| benchmark | median | throughput |
| --- | ---: | ---: |
| cold_start/by_hand/new_app | 85.7 ns |  |
| cold_start/single_thread/new | 3.04 ns |  |
| cold_start/single_thread/new_and_first_handler | 89.8 ns |  |
| cold_start/thread_safe/new | 3.91 ns |  |
| cold_start/thread_safe/new_and_first_handler | 103 ns |  |
| compare/cold/baseline | 105 ns |  |
| compare/cold/dill | 2.69 µs |  |
| compare/cold/injecta | 102 ns |  |
| compare/cold/nject | 172 ns |  |
| compare/cold/shaku | 1.63 µs |  |
| compare/cold/teloc | 207 ns |  |
| compare/singleton/baseline | 5.39 ns |  |
| compare/singleton/dill | 56.5 ns |  |
| compare/singleton/injecta | 5.34 ns |  |
| compare/singleton/nject | 7.83 ns |  |
| compare/singleton/shaku | 7.99 ns |  |
| compare/singleton/teloc | 7.07 ns |  |
| compare/transient_graph/baseline | 11 ns |  |
| compare/transient_graph/dill | 628 ns |  |
| compare/transient_graph/injecta | 11.9 ns |  |
| compare/transient_graph/nject | 11.2 ns |  |
| compare/transient_graph/shaku | 217 ns |  |
| compare/transient_graph/teloc | 26.7 ns |  |
| concurrent/handler_by_hand/1 | 121 µs | 82.4 M/s |
| concurrent/handler_by_hand/2 | 1.27 ms | 15.7 M/s |
| concurrent/handler_by_hand/4 | 6.01 ms | 6.7 M/s |
| concurrent/handler_by_hand/8 | 14.4 ms | 5.5 M/s |
| concurrent/handler_resolve/1 | 145 µs | 69.0 M/s |
| concurrent/handler_resolve/2 | 1.37 ms | 14.6 M/s |
| concurrent/handler_resolve/4 | 6.09 ms | 6.6 M/s |
| concurrent/handler_resolve/8 | 19 ms | 4.2 M/s |
| concurrent/singleton_by_hand/1 | 62.6 µs | 159.8 M/s |
| concurrent/singleton_by_hand/2 | 281 µs | 71.2 M/s |
| concurrent/singleton_by_hand/4 | 1.24 ms | 32.2 M/s |
| concurrent/singleton_by_hand/8 | 3.78 ms | 21.2 M/s |
| concurrent/singleton_get/1 | 17 µs | 589.1 M/s |
| concurrent/singleton_get/2 | 34.1 µs | 586.5 M/s |
| concurrent/singleton_get/4 | 40.3 µs | 991.9 M/s |
| concurrent/singleton_get/8 | 76.3 µs | 1,048.1 M/s |
| concurrent/singleton_resolve/1 | 57.1 µs | 175.0 M/s |
| concurrent/singleton_resolve/2 | 265 µs | 75.6 M/s |
| concurrent/singleton_resolve/4 | 1.17 ms | 34.0 M/s |
| concurrent/singleton_resolve/8 | 3.77 ms | 21.2 M/s |
| deep_chain/by_hand/10 | 0.574 ns |  |
| deep_chain/by_hand/20 | 1.24 ns |  |
| deep_chain/by_hand/50 | 1.17 ns |  |
| deep_chain/injecta/10 | 0.582 ns |  |
| deep_chain/injecta/20 | 1.16 ns |  |
| deep_chain/injecta/50 | 1.33 ns |  |
| loop/by_hand/handler_x1000 | 12 µs | 83.4 M/s |
| loop/injecta/handler_x1000 | 12.7 µs | 79.0 M/s |
| loop/injecta/singleton_get_x1000 | 628 ns | 1,592.9 M/s |
| loop/injecta/singleton_resolve_x1000 | 5.27 µs | 189.8 M/s |
| override/new | 3.7 ns |  |
| override/new_with_fake_logger | 5.56 ns |  |
| override/new_with_fake_logger_first_handler | 81 ns |  |
| scope/cached_scoped_get | 0.733 ns |  |
| scope/cached_scoped_resolve | 5.21 ns |  |
| scope/open_resolve_handler_drop | 40.3 ns |  |
| scope/open_resolve_scoped_drop | 32.9 ns |  |
| scope/root_singleton_via_scope | 5.54 ns |  |
| scope/root_transient_in_scope | 11.5 ns |  |
| singleton/single_thread/cached_get | 0.673 ns |  |
| singleton/single_thread/cached_resolve | 5.52 ns |  |
| singleton/single_thread/first_resolve | 37.7 ns |  |
| singleton/thread_safe/cached_get | 0.756 ns |  |
| singleton/thread_safe/cached_resolve | 5.65 ns |  |
| singleton/thread_safe/first_resolve | 48.6 ns |  |
| transient/by_hand/handler_checksum | 12.2 ns |  |
| transient/single_thread/handler | 14.4 ns |  |
| transient/thread_safe/handler | 14.1 ns |  |
| transient/thread_safe/handler_checksum | 12.6 ns |  |
