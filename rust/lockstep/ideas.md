# Ideas

- Async locks. The root token is per thread, and tasks move between threads, so async code
  needs a per-task token instead (happylock names this its main open problem). Out of scope
  until the sync API is done; see `docs/research.md`, decision 10.
