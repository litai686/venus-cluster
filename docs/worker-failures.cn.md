## Sealing Failures inside venus-worker
我们将 `sealing` 过程中可能产生的错误类型归为四个级别，并设定了不同的处理方式：
- temp(orary)，临时:
  这个级别的错误通常明确属于临时性的，或者我们知道重试不会带来负面影响的。
  对于这类错误，`worker` 会自动尝试重试（以 `recover_interval` 为间隔，最多 `max_retries` 次）。
  当重试次数超过设定的上限时，会自动升级到 `perm` 级别。
  RPC 错误是比较典型的临时错误类型。

- perm(anent)，持续:
  这个级别的错误通常出现在 `sealing` 的过程中，无法简单判断是否可以安全重试，或一旦修复会有较大的收益（如不必重新完成 pre commit phase1）。
  这类错误会阻塞 `worker` 线程，直到人工介入完成处理。

- crit(tical)，严重:
  严重级别的错误在各个方面都与持续级别的错误比较相似。
  比较显著的区别是，严重级别的错误通常明确来自运行的环境而非 `sealing` 的过程。
  如可甄别的本地文件系统异常、本地持久化数据库异常等都归入此类。
  严重级别的错误同样也会阻塞 `worker` 线程直到人工介入。


- abort，终止:
  遇到这个级别的错误，`worker` 会直接放弃当前的 `sealing` 进度，尝试重新开始一个新的流程。