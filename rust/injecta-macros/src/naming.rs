//! Derives readable parameter names (`App::new(config, user_id)`) from the
//! registered types, so the generated constructor documents itself.

use syn::{GenericArgument, PathArguments, Type, TypeParamBound};

/// `Arc<dyn UserRepo>` -> `user_repo`, `Config` -> `config`; `value` when no
/// name can be read off the type.
pub(crate) fn param_name(ty: &Type) -> String {
    ident_of(ty).map_or_else(|| String::from("value"), |ident| snake_case(&ident))
}

fn ident_of(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => {
            let last = path.path.segments.last()?;
            let name = last.ident.to_string();
            if matches!(name.as_str(), "Arc" | "Box" | "Rc") {
                if let PathArguments::AngleBracketed(args) = &last.arguments {
                    if let Some(GenericArgument::Type(inner)) = args.args.first() {
                        return ident_of(inner);
                    }
                }
            }
            Some(name)
        }
        Type::TraitObject(object) => object.bounds.iter().find_map(|bound| match bound {
            TypeParamBound::Trait(t) => t.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        }),
        Type::Paren(inner) => ident_of(&inner.elem),
        Type::Group(inner) => ident_of(&inner.elem),
        _ => None,
    }
}

/// Renders a type the way it is usually written (`Arc<dyn Logger>`), not with
/// the token spacing of `to_token_stream` (`Arc < dyn Logger >`).
pub(crate) fn type_display(ty: &Type) -> String {
    let mut out = quote::ToTokens::to_token_stream(ty).to_string();
    for (from, to) in [
        (" <", "<"),
        ("< ", "<"),
        (" >", ">"),
        (" ,", ","),
        (" ::", "::"),
        (":: ", "::"),
        ("& ", "&"),
        ("( ", "("),
        (" )", ")"),
    ] {
        out = out.replace(from, to);
    }
    out
}

fn snake_case(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len() + camel.len() / 2);
    let mut prev_lower = false;
    for ch in camel.chars() {
        if ch.is_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
        }
    }
    out
}

/// Makes every name unique by suffixing repeats with `_2`, `_3`, ...
pub(crate) fn dedup(names: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(names.len());
    names
        .into_iter()
        .map(|name| {
            let mut candidate = name.clone();
            let mut n = 1;
            while seen.contains(&candidate) {
                n += 1;
                candidate = format!("{name}_{n}");
            }
            seen.push(candidate.clone());
            candidate
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{dedup, param_name};

    fn name(src: &str) -> String {
        param_name(&syn::parse_str(src).unwrap())
    }

    #[test]
    fn names_follow_the_innermost_type() {
        assert_eq!(name("Config"), "config");
        assert_eq!(name("UserId"), "user_id");
        assert_eq!(name("std::sync::Arc<dyn UserRepo + Send>"), "user_repo");
        assert_eq!(name("Box<HttpClient>"), "http_client");
        assert_eq!(name("(u32, u8)"), "value");
    }

    #[test]
    fn type_display_keeps_spaces_only_between_words() {
        let ty = syn::parse_str("std::sync::Arc<dyn Logger + Send>").unwrap();
        assert_eq!(
            super::type_display(&ty),
            "std::sync::Arc<dyn Logger + Send>"
        );
    }

    #[test]
    fn dedup_suffixes_repeated_names() {
        let names = vec!["db".to_owned(), "db".to_owned(), "x".to_owned()];
        assert_eq!(dedup(names), ["db", "db_2", "x"]);
    }
}
