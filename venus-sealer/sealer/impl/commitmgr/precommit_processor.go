package commitmgr

import (
	"bytes"
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/filecoin-project/go-address"
	"github.com/filecoin-project/go-state-types/abi"
	"github.com/filecoin-project/go-state-types/big"
	"github.com/filecoin-project/specs-actors/v5/actors/builtin"
	builtin5 "github.com/filecoin-project/specs-actors/v5/actors/builtin"
	"github.com/filecoin-project/specs-actors/v5/actors/builtin/miner"
	specactors "github.com/filecoin-project/venus/pkg/specactors/builtin/miner"

	"github.com/dtynn/venus-cluster/venus-sealer/pkg/messager"
	"github.com/dtynn/venus-cluster/venus-sealer/sealer/api"
)

type PreCommitProcessor struct {
	api       SealingAPI
	msgClient messager.API

	smgr api.SectorStateManager

	config Cfg
}

func (p PreCommitProcessor) processIndividually(ctx context.Context, sectors []api.SectorState, from address.Address, mid abi.ActorID) {
	var spec messager.MsgMeta
	cmcfg := p.config.commitment(mid)
	spec.GasOverEstimation = cmcfg.PreCommitGasOverEstimation
	spec.MaxFeeCap = cmcfg.MaxPreCommitFeeCap

	wg := sync.WaitGroup{}
	wg.Add(len(sectors))
	for i := range sectors {
		go func(idx int) {
			defer wg.Done()

			params, deposit, _, err := preCommitParams(ctx, p.api, sectors[idx])
			if err != nil {
				log.Error("get pre-commit params failed: ", err)
				return
			}
			enc := new(bytes.Buffer)
			if err := params.MarshalCBOR(enc); err != nil {
				log.Error("serialize pre-commit sector parameters failed: ", err)
				return
			}

			mcid, err := pushMessage(ctx, from, mid, deposit, specactors.Methods.ProveCommitSector, p.msgClient, spec, enc.Bytes())
			if err != nil {
				log.Error("push pre-commit single failed: ", err)
				return
			}
			log.Infof("precommit of sector %d sent cid: %s", sectors[idx].ID.Number, mcid)

			sectors[idx].MessageInfo.PreCommitCid = &mcid
		}(i)
	}
	wg.Wait()
}

func (p PreCommitProcessor) Process(ctx context.Context, sectors []api.SectorState, mid abi.ActorID) error {
	// Notice: If a sector in sectors has been sent, it's cid failed should be changed already.
	defer p.cleanSector(ctx, sectors)

	from := p.config.commitment(mid).PreCommitControlAddress
	if !p.EnableBatch(mid) {
		p.processIndividually(ctx, sectors, from, mid)
		return nil
	}
	infos := []api.PreCommitEntry{}
	failed := map[abi.SectorID]struct{}{}
	for _, s := range sectors {
		params, deposit, _, err := preCommitParams(ctx, p.api, s)
		if err != nil {
			log.Errorf("get precommit %s %d params failed: %s\n", s.ID.Miner, s.ID.Number, err)
			failed[s.ID] = struct{}{}
			continue
		}
		infos = append(infos, api.PreCommitEntry{
			Deposit: deposit,
			Pci:     params,
		})
	}
	params := miner.PreCommitSectorBatchParams{}

	deposit := big.Zero()
	for i := range infos {
		params.Sectors = append(params.Sectors, *infos[i].Pci)
		deposit = big.Add(deposit, infos[i].Deposit)
	}

	enc := new(bytes.Buffer)
	if err := params.MarshalCBOR(enc); err != nil {
		return fmt.Errorf("couldn't serialize PreCommitSectorBatchParams: %w", err)
	}
	var spec messager.MsgMeta
	cmcfg := p.config.commitment(mid)
	spec.GasOverEstimation = cmcfg.BatchProCommitGasOverEstimation
	spec.MaxFeeCap = cmcfg.MaxBatchProCommitFeeCap

	ccid, err := pushMessage(ctx, from, mid, deposit, builtin5.MethodsMiner.PreCommitSectorBatch,
		p.msgClient, spec, enc.Bytes())
	if err != nil {
		return fmt.Errorf("push batch precommit message failed: %w", err)
	}
	for i := range sectors {
		if _, ok := failed[sectors[i].ID]; !ok {
			sectors[i].MessageInfo.PreCommitCid = &ccid
		}
	}
	return nil
}

func (p PreCommitProcessor) Expire(ctx context.Context, sectors []api.SectorState, mid abi.ActorID) (map[abi.SectorID]struct{}, error) {
	maxWait := p.config.commitment(mid).PreCommitBatchMaxWait
	maxWaitHeight := abi.ChainEpoch(maxWait / (builtin.EpochDurationSeconds * time.Second))
	_, h, err := p.api.ChainHead(ctx)
	if err != nil {
		return nil, err
	}

	expire := map[abi.SectorID]struct{}{}
	for _, s := range sectors {
		if h-s.Ticket.Epoch > maxWaitHeight {
			expire[s.ID] = struct{}{}
		}
	}

	return expire, nil
}

func (p PreCommitProcessor) CheckAfter(mid abi.ActorID) *time.Timer {
	return time.NewTimer(p.config.commitment(mid).PreCommitCheckInterval)
}

func (p PreCommitProcessor) Threshold(mid abi.ActorID) int {
	return p.config.commitment(mid).PreCommitBatchThreshold
}

func (p PreCommitProcessor) EnableBatch(mid abi.ActorID) bool {
	return p.config.commitment(mid).EnableBatchPreCommit
}

func (p PreCommitProcessor) cleanSector(ctx context.Context, sector []api.SectorState) {
	for i := range sector {
		sector[i].MessageInfo.NeedSend = false
		err := p.smgr.Update(ctx, sector[i].ID, &sector[i].MessageInfo)
		if err != nil {
			log.Errorf("Update sector %s MessageInfo failed: ", err)
		}
	}
}

var _ Processor = (*PreCommitProcessor)(nil)
