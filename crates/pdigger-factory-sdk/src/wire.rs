//! String enums of the wire format that stay forward compatible: every value this version of the
//! SDK does not know parses as `Other(..)` and prints back unchanged.

/// Common surface of the SDK's string enums (implemented by `wire_enum!`).
pub trait WireEnum: Sized + Clone + PartialEq + std::fmt::Debug + 'static {
    /// The values this SDK version knows, in the spec's order.
    fn known() -> &'static [Self];
    /// The wire name.
    fn as_str(&self) -> &str;
    /// The value for a wire name; unknown names give the `Other` variant.
    fn parse(name: &str) -> Self;
}

/// Defines a string enum with an `Other(String)` variant, its wire names, `Display`, `Default`
/// (the first value), `From<&str>` and serde in both directions.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $var:ident = $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($(#[$vmeta])* $var,)+
            /// A value this version of the SDK does not know, as sent.
            Other(String),
        }

        impl $name {
            const KNOWN: &'static [$name] = &[$($name::$var),+];
        }

        impl $crate::wire::WireEnum for $name {
            fn known() -> &'static [Self] {
                Self::KNOWN
            }
            fn as_str(&self) -> &str {
                match self {
                    $($name::$var => $wire,)+
                    $name::Other(other) => other,
                }
            }
            fn parse(name: &str) -> Self {
                match name {
                    $($wire => $name::$var,)+
                    other => $name::Other(other.to_string()),
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::KNOWN[0].clone()
            }
        }

        impl From<&str> for $name {
            fn from(name: &str) -> Self {
                <Self as $crate::wire::WireEnum>::parse(name)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str($crate::wire::WireEnum::as_str(self))
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str($crate::wire::WireEnum::as_str(self))
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let name = String::deserialize(deserializer)?;
                Ok(<Self as $crate::wire::WireEnum>::parse(&name))
            }
        }
    };
}

pub(crate) use wire_enum;

#[cfg(test)]
pub(crate) mod tests {
    use super::WireEnum;
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    /// Every known value round-trips through its name, `Display`, `From<&str>` and JSON; an
    /// unknown name is kept as `Other`; the default is the first known value.
    pub(crate) fn round_trip<T>()
    where
        T: WireEnum + Serialize + DeserializeOwned + std::fmt::Display + Default + for<'a> From<&'a str>,
    {
        assert!(!T::known().is_empty());
        for value in T::known() {
            let name = value.as_str();
            assert_eq!(&T::parse(name), value);
            assert_eq!(&T::from(name), value);
            assert_eq!(value.to_string(), name);
            let json = serde_json::to_string(value).unwrap();
            assert_eq!(json, format!("\"{name}\""));
            assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value);
        }
        let other = T::parse("zz-not-a-value");
        assert_eq!(other.as_str(), "zz-not-a-value");
        assert!(!T::known().contains(&other));
        assert_eq!(&T::default(), &T::known()[0]);
        assert!(serde_json::from_str::<T>("3").is_err());
    }
}
