Interleaved runs: 10 rounds x 4 trials per variant and workload (n = 40), pin = 2,4, count = 1000000, capacity = 8192.

### u64

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 7.111 ms | [6.508, 7.822] | 5.174 ms | 21.141 ms | baseline |  |  |
| rust_unchecked | 40 | 7.614 ms | [7.138, 7.922] | 5.662 ms | 8.449 ms | 1.071 [0.939, 1.178] | 0.6615 | +0.058 |
| rust_before_fix | 40 | 6.451 ms | [5.420, 6.780] | 3.135 ms | 7.124 ms | 0.907 [0.748, 1.001] | 0.0085 | -0.343 |
| cpp_gcc | 40 | 7.760 ms | [6.855, 9.182] | 4.979 ms | 9.869 ms | 1.091 [0.940, 1.346] | 0.9501 | +0.009 |

### order

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 14.142 ms | [12.944, 15.310] | 11.278 ms | 15.352 ms | baseline |  |  |
| rust_unchecked | 40 | 14.014 ms | [12.509, 15.570] | 10.373 ms | 19.756 ms | 0.991 [0.871, 1.139] | 0.8663 | -0.022 |
| rust_before_fix | 40 | 15.329 ms | [14.106, 16.307] | 11.859 ms | 19.948 ms | 1.084 [0.968, 1.207] | 0.0331 | +0.278 |
| cpp_gcc | 40 | 16.609 ms | [15.168, 17.875] | 12.918 ms | 21.333 ms | 1.174 [1.043, 1.323] | 0.0014 | +0.415 |

### latency

| variant | n | median | 95% CI (bootstrap) | min | mean | vs rust: ratio [CI] | Mann-Whitney p | Cliff's delta |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | 40 | 2.679 ns | [2.300, 3.023] | 1.438 ns | 3.076 ns | baseline |  |  |
| rust_unchecked | 40 | 1.909 ns | [1.673, 2.363] | 1.199 ns | 2.194 ns | 0.712 [0.588, 0.949] | 0.0012 | -0.421 |
| rust_before_fix | 40 | 3.403 ns | [3.252, 3.588] | 2.434 ns | 3.821 ns | 1.270 [1.124, 1.490] | 4.6e-05 | +0.530 |
| cpp_gcc | 40 | 1.577 ns | [1.342, 1.672] | 1.111 ns | 1.692 ns | 0.589 [0.491, 0.692] | 5.7e-09 | -0.757 |

