Interleaved runs: 10 rounds x 4 trials per variant and workload (n = 40), pin = 2,4, count = 1000000, capacity = 8192.

### u64

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 5.458 ms | [5.150, 5.826] | 2.708 ms | 5.910 ms | baseline |  |  |
| rust_before_fix | 40 | 5.703 ms | [5.263, 5.861] | 2.811 ms | 5.608 ms | 1.045 [0.949, 1.120] | 0.8966 | +0.018 |
| cpp_gcc | 40 | 5.616 ms | [5.454, 5.774] | 4.721 ms | 5.808 ms | 1.029 [0.960, 1.104] | 0.3146 | +0.131 |

### order

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 12.553 ms | [12.254, 12.847] | 11.360 ms | 13.370 ms | baseline |  |  |
| rust_before_fix | 40 | 14.494 ms | [13.918, 15.178] | 12.199 ms | 14.955 ms | 1.155 [1.102, 1.214] | 2.3e-06 | +0.615 |
| cpp_gcc | 40 | 14.080 ms | [13.561, 14.912] | 11.989 ms | 14.383 ms | 1.122 [1.074, 1.189] | 8.5e-05 | +0.511 |

### latency

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 1.610 ns | [1.546, 1.708] | 1.413 ns | 1.674 ns | baseline |  |  |
| rust_before_fix | 40 | 2.752 ns | [2.546, 3.083] | 2.317 ns | 2.889 ns | 1.710 [1.552, 1.916] | 1.4e-14 | +1.000 |
| cpp_gcc | 40 | 1.196 ns | [1.159, 1.236] | 0.986 ns | 1.257 ns | 0.743 [0.691, 0.781] | 3.9e-10 | -0.814 |

