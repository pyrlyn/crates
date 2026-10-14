//! ULID-backed newtype identifiers for the two ids a provider request names:
//! a tool call (`CallId`) and an archived tool output (`ArchiveId`). Each is a
//! newtype instead of a bare `String`, so a mixed-up id is a compile error,
//! and serializes as its ULID string form.

/// What [`ulid_id!`] expands to, re-exported so a crate that calls the macro
/// needs no `ulid`, `serde` or `schemars` dependency of its own.
#[doc(hidden)]
pub mod __private {
    pub use schemars;
    pub use serde;
    pub use ulid;
}

/// Declares a ULID newtype with `new`, `Display`, `FromStr`, string-shaped
/// serde and a JSON Schema, so every id gets identical behaviour. Exported so
/// a crate with ids of its own (a turn, a transcript item) shares this one
/// definition instead of copying it.
#[macro_export]
macro_rules! ulid_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($crate::ids::__private::ulid::Ulid);

        impl $name {
            /// Generates a fresh, time-sortable id.
            pub fn new() -> Self {
                // ulid 3.x renamed `Ulid::new()` to `Ulid::generate()`; the
                // wrapper keeps its name so callers do not churn with the dep.
                Self($crate::ids::__private::ulid::Ulid::generate())
            }
        }

        impl ::std::default::Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::ids::__private::ulid::DecodeError;

            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                Ok(Self(
                    <$crate::ids::__private::ulid::Ulid as ::std::str::FromStr>::from_str(s)?,
                ))
            }
        }

        // Written out instead of `#[serde(transparent)]`: the derive would
        // name `serde` in the calling crate, which may not depend on it.
        impl $crate::ids::__private::serde::Serialize for $name {
            fn serialize<S: $crate::ids::__private::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error> {
                $crate::ids::__private::serde::Serialize::serialize(&self.0, serializer)
            }
        }

        impl<'de> $crate::ids::__private::serde::Deserialize<'de> for $name {
            fn deserialize<D: $crate::ids::__private::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                $crate::ids::__private::serde::Deserialize::deserialize(deserializer).map(Self)
            }
        }

        impl $crate::ids::__private::schemars::JsonSchema for $name {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(stringify!($name))
            }

            fn json_schema(
                _gen: &mut $crate::ids::__private::schemars::SchemaGenerator,
            ) -> $crate::ids::__private::schemars::Schema {
                $crate::ids::__private::schemars::json_schema!({
                    "type": "string",
                    "description": concat!(stringify!($name), ": a 26-character Crockford-base32 ULID."),
                    "pattern": "^[0-7][0-9A-HJKMNP-TV-Z]{25}$"
                })
            }
        }
    };
}

ulid_id!(
    CallId,
    "Identifies one tool call, from `ToolCallRequested` to `ToolCallDone`."
);
ulid_id!(
    ArchiveId,
    "Identifies one archived (pre-truncation) tool output row."
);

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn ulid_id_roundtrips_through_display_and_from_str() {
        let id = ArchiveId::new();
        let parsed: ArchiveId = id.to_string().parse().expect("display form parses back");
        assert_eq!(id, parsed);
    }

    #[test]
    fn ulid_id_roundtrips_through_json() {
        let id = CallId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, format!("\"{id}\""));
        let back: CallId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, back);
    }

    #[test]
    fn distinct_id_types_are_distinct_types() {
        // Compile-time proof only: this would not type-check if the macro
        // produced interchangeable ids.
        fn takes_call_id(_: CallId) {}
        takes_call_id(CallId::new());
    }
}
