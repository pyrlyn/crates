# path-gates — completed tasks

### T1. Map changed paths to named gates by glob rules

`Rules::from_toml` parses `[[gate]]` tables (`name`, `paths`, `always`) and an `unmatched = "all" | "ignore"` policy; `Rules::select` returns a `Selection` with the chosen gates, the unclaimed paths, and `all` when an unclaimed path forced every gate. `Rules::new` builds the same rules from `GateSpec` values without TOML, and unknown keys are ignored so a host tool can share the tables. Duplicate gate names and invalid globs are config errors.
