# Ideas

- Close an interrupted reply's open items (`ItemDone`) before `TurnDone`, and keep the words a voice user already heard as a partial assistant message; cox leaves both to the surface today.
- A `State` change event, so a surface shows "waiting for approval" without polling `Agent::state`.
