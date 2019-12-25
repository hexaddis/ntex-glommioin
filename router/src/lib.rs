//! Resource path matching library.
mod de;
mod path;
mod resource;
mod router;

pub use self::de::PathDeserializer;
pub use self::path::Path;
pub use self::resource::ResourceDef;
pub use self::router::{ResourceInfo, Router, RouterBuilder};

pub trait Resource<T: ResourcePath> {
    fn resource_path(&mut self) -> &mut Path<T>;
}

pub trait ResourcePath {
    fn path(&self) -> &str;
}

impl ResourcePath for String {
    fn path(&self) -> &str {
        self.as_str()
    }
}

impl<'a> ResourcePath for &'a str {
    fn path(&self) -> &str {
        self
    }
}

impl ResourcePath for bytestring::ByteString {
    fn path(&self) -> &str {
        &*self
    }
}

/// Helper trait for type that could be converted to path pattern
pub trait IntoPatterns {
    /// Signle patter
    fn is_single(&self) -> bool;

    fn patterns(&self) -> Vec<String>;
}

impl IntoPatterns for String {
    fn is_single(&self) -> bool {
        true
    }

    fn patterns(&self) -> Vec<String> {
        vec![self.clone()]
    }
}

impl<'a> IntoPatterns for &'a str {
    fn is_single(&self) -> bool {
        true
    }

    fn patterns(&self) -> Vec<String> {
        vec![self.to_string()]
    }
}

impl<T: AsRef<str>> IntoPatterns for Vec<T> {
    fn is_single(&self) -> bool {
        self.len() == 1
    }

    fn patterns(&self) -> Vec<String> {
        self.into_iter().map(|v| v.as_ref().to_string()).collect()
    }
}

#[cfg(feature = "http")]
mod url;

#[cfg(feature = "http")]
pub use self::url::{Quoter, Url};

#[cfg(feature = "http")]
mod http_support {
    use super::ResourcePath;
    use http::Uri;

    impl ResourcePath for Uri {
        fn path(&self) -> &str {
            self.path()
        }
    }
}
