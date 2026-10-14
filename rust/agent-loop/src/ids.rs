//! ULID-backed ids for what the loop itself names: a turn (`TurnId`) and a
//! transcript item (`ItemId`). Tool calls reuse `llm_wire::CallId`, which
//! the provider stream already carries. Newtypes, so a mixed-up id is a
//! compile error; each serializes as its ULID string.

use llm_wire::ulid_id;

ulid_id!(
    TurnId,
    "Identifies one user turn, from `TurnStarted` to `TurnDone`."
);
ulid_id!(
    ItemId,
    "Identifies one transcript item, from `ItemStarted` to `ItemDone`."
);

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn ids_roundtrip_through_display_and_json() {
        let id = TurnId::new();
        let parsed: TurnId = id.to_string().parse().expect("display form parses back");
        assert_eq!(id, parsed);
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, format!("\"{id}\""));
    }
}
