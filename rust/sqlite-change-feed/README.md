# sqlite-change-feed

Notice when another process commits to a SQLite database you share, through
Diesel:

```rust
use sqlite_change_feed::ChangeToken;

let mut token = ChangeToken::new(&mut conn)?;
// ... later, on each refresh:
if token.poll(&mut conn)? {
    // another connection committed: re-read what you show
}
```

It polls `PRAGMA data_version`, which moves when another connection commits
and stays put for your own commits. It does not say which table changed.
No thread and no timer: poll as often as you refresh. A token belongs to the
connection that issued it; each consumer keeps its own.

The crate enables only Diesel's `sqlite` feature and no `libsqlite3-sys`
features, so the application chooses bundled SQLite, the system library or
SQLCipher.

Extracted from cox's `cox-store` (`watch.rs`).

Licensed under either of MIT or Apache-2.0 at your option.
