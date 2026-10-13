//! Static description of a container (its entries and their lifetimes) for
//! diagnostics and agents. Generated as `const` data, so it adds no runtime
//! work and can be inspected without building any value.

use core::fmt;

/// How long a resolved value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Lifetime {
    /// Passed to the constructor; every resolve returns a clone.
    Instance,
    /// Created on first resolve, cached for the container's life; resolves
    /// return clones (use `Arc<T>` to share).
    Singleton,
    /// Like `Singleton`, but cached per scope value.
    Scoped,
    /// Built from its dependencies on every resolve.
    Transient,
}

impl fmt::Display for Lifetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `pad`, not `write_str`, so width specifiers like `{:<9}` apply.
        f.pad(match self {
            Self::Instance => "instance",
            Self::Singleton => "singleton",
            Self::Scoped => "scoped",
            Self::Transient => "transient",
        })
    }
}

/// One container entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProviderInfo {
    /// The registered type as written in `container!`.
    pub type_name: &'static str,
    /// Its lifetime.
    pub lifetime: Lifetime,
    /// Whether the entry is built by an explicit `= |c| ...` factory.
    pub factory: bool,
}

/// Implemented by every `container!` type.
pub trait Container {
    /// The container's name as declared.
    const NAME: &'static str;
    /// Its own entries, in declaration order (a scope lists only its own).
    const PROVIDERS: &'static [ProviderInfo];

    /// A printable list of the entries, for logs and debugging.
    #[must_use]
    fn describe() -> Description {
        Description {
            name: Self::NAME,
            providers: Self::PROVIDERS,
        }
    }
}

/// Display adapter returned by [`Container::describe`].
#[derive(Debug, Clone, Copy)]
pub struct Description {
    name: &'static str,
    providers: &'static [ProviderInfo],
}

impl fmt::Display for Description {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.name)?;
        for info in self.providers {
            let factory = if info.factory { " (factory)" } else { "" };
            writeln!(f, "  {:<9} {}{factory}", info.lifetime, info.type_name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Container, Lifetime, ProviderInfo};
    use alloc::string::ToString;

    struct Fake;

    impl Container for Fake {
        const NAME: &'static str = "Fake";
        const PROVIDERS: &'static [ProviderInfo] = &[
            ProviderInfo {
                type_name: "Config",
                lifetime: Lifetime::Instance,
                factory: false,
            },
            ProviderInfo {
                type_name: "Arc<dyn Log>",
                lifetime: Lifetime::Singleton,
                factory: true,
            },
        ];
    }

    #[test]
    fn describe_lists_every_entry_with_lifetime() {
        let text = Fake::describe().to_string();
        assert_eq!(
            text,
            "Fake\n  instance  Config\n  singleton Arc<dyn Log> (factory)\n"
        );
    }
}
