# Ideas

- aulo's bash tool classifies its command lines through this crate.
- `Write` is never returned today: a file write is `Exec`. Split `echo hi > out.txt` and `tee` into `Write` once an adopter wants the difference.
