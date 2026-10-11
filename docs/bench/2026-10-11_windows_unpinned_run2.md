Interleaved runs: 10 rounds x 4 trials per variant and workload (n = 40), pin = none, count = 1000000, capacity = 8192.

### u64

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 4.127 ms | [3.638, 4.633] | 2.361 ms | 4.235 ms | baseline |  |  |
| rust_unchecked | 40 | 4.958 ms | [4.635, 5.380] | 2.321 ms | 4.996 ms | 1.201 [1.052, 1.409] | 0.0034 | +0.381 |
| rust_before_fix | 40 | 3.998 ms | [3.882, 4.349] | 2.661 ms | 4.499 ms | 0.969 [0.864, 1.128] | 0.9578 | +0.007 |
| cpp_gcc | 40 | 4.943 ms | [4.714, 5.287] | 2.377 ms | 5.136 ms | 1.198 [1.059, 1.377] | 0.0004 | +0.464 |

### order

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 11.682 ms | [11.464, 12.103] | 8.390 ms | 12.068 ms | baseline |  |  |
| rust_unchecked | 40 | 11.546 ms | [11.104, 12.259] | 7.769 ms | 12.295 ms | 0.988 [0.943, 1.045] | 0.5476 | -0.079 |
| rust_before_fix | 40 | 12.056 ms | [11.296, 12.732] | 6.222 ms | 11.813 ms | 1.032 [0.961, 1.090] | 0.8062 | +0.033 |
| cpp_gcc | 40 | 12.920 ms | [12.214, 13.706] | 7.640 ms | 13.061 ms | 1.106 [1.039, 1.180] | 0.0031 | +0.385 |

### latency

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 2.075 ns | [1.907, 2.463] | 1.428 ns | 2.297 ns | baseline |  |  |
| rust_unchecked | 40 | 1.942 ns | [1.562, 2.187] | 1.157 ns | 1.945 ns | 0.936 [0.702, 1.091] | 0.0448 | -0.261 |
| rust_before_fix | 40 | 2.973 ns | [2.796, 3.329] | 2.329 ns | 3.117 ns | 1.433 [1.196, 1.675] | 2.7e-07 | +0.669 |
| cpp_gcc | 40 | 1.365 ns | [1.218, 1.560] | 1.031 ns | 1.591 ns | 0.658 [0.528, 0.771] | 9.0e-07 | -0.639 |

