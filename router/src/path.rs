use std::ops::Index;
use std::rc::Rc;

use serde::de;

use crate::de::PathDeserializer;
use crate::{Resource, ResourcePath};

#[derive(Debug, Clone, Copy)]
pub(crate) enum PathItem {
    Static(&'static str),
    Segment(u16, u16),
}

/// Resource path match information
///
/// If resource path contains variable patterns, `Path` stores them.
#[derive(Debug)]
pub struct Path<T> {
    path: T,
    pub(crate) skip: u16,
    pub(crate) segments: Vec<(Rc<String>, PathItem)>,
}

impl<T: Default> Default for Path<T> {
    fn default() -> Self {
        Path {
            path: T::default(),
            skip: 0,
            segments: Vec::new(),
        }
    }
}

impl<T: Clone> Clone for Path<T> {
    fn clone(&self) -> Self {
        Path {
            path: self.path.clone(),
            skip: self.skip,
            segments: self.segments.clone(),
        }
    }
}

impl<T: ResourcePath> Path<T> {
    pub fn new(path: T) -> Path<T> {
        Path {
            path,
            skip: 0,
            segments: Vec::new(),
        }
    }

    /// Get reference to inner path instance
    pub fn get_ref(&self) -> &T {
        &self.path
    }

    /// Get mutable reference to inner path instance
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.path
    }

    /// Path
    pub fn path(&self) -> &str {
        let skip = self.skip as usize;
        let path = self.path.path();
        if skip <= path.len() {
            &path[skip..]
        } else {
            ""
        }
    }

    /// Reset inner path
    pub fn set(&mut self, path: T) {
        self.skip = 0;
        self.path = path;
        self.segments.clear();
    }

    /// Skip first `n` chars in path
    pub fn skip(&mut self, n: u16) {
        self.skip = self.skip + n;
    }

    pub(crate) fn add(&mut self, name: Rc<String>, value: PathItem) {
        match value {
            PathItem::Static(s) => self.segments.push((name, PathItem::Static(s))),
            PathItem::Segment(begin, end) => self
                .segments
                .push((name, PathItem::Segment(self.skip + begin, self.skip + end))),
        }
    }

    #[doc(hidden)]
    pub fn add_static(&mut self, name: &str, value: &'static str) {
        self.segments
            .push((Rc::new(name.to_string()), PathItem::Static(value)));
    }

    /// Check if there are any matched patterns
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Check number of extracted parameters
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Get matched parameter by name without type conversion
    pub fn get(&self, key: &str) -> Option<&str> {
        for item in self.segments.iter() {
            if key == item.0.as_str() {
                return match item.1 {
                    PathItem::Static(ref s) => Some(&s),
                    PathItem::Segment(s, e) => {
                        Some(&self.path.path()[(s as usize)..(e as usize)])
                    }
                };
            }
        }
        if key == "tail" {
            Some(&self.path.path()[(self.skip as usize)..])
        } else {
            None
        }
    }

    /// Get unprocessed part of the path
    pub fn unprocessed(&self) -> &str {
        &self.path.path()[(self.skip as usize)..]
    }

    /// Get matched parameter by name.
    ///
    /// If keyed parameter is not available empty string is used as default
    /// value.
    pub fn query(&self, key: &str) -> &str {
        if let Some(s) = self.get(key) {
            s
        } else {
            ""
        }
    }

    /// Return iterator to items in parameter container
    pub fn iter(&self) -> PathIter<T> {
        PathIter {
            idx: 0,
            params: self,
        }
    }

    /// Try to deserialize matching parameters to a specified type `U`
    pub fn load<'de, U: serde::Deserialize<'de>>(&'de self) -> Result<U, de::value::Error> {
        de::Deserialize::deserialize(PathDeserializer::new(self))
    }
}

#[cfg(feature = "http")]
use std::borrow::Cow;

#[cfg(feature = "http")]
impl Path<crate::Url> {
    /// Get URL-decoded matched parameter by name without type conversion
    pub fn get_decoded(&self, key: &str) -> Option<Cow<str>> {
        use crate::url::RESERVED_QUOTER;

        self.get(key).map(|value| {
            if let Some(value) = RESERVED_QUOTER.with(|q| q.requote(value.as_bytes())) {
                Cow::Owned(value)
            } else {
                Cow::Borrowed(value)
            }
        })
    }
}

#[derive(Debug)]
pub struct PathIter<'a, T> {
    idx: usize,
    params: &'a Path<T>,
}

impl<'a, T: ResourcePath> Iterator for PathIter<'a, T> {
    type Item = (&'a str, &'a str);

    #[inline]
    fn next(&mut self) -> Option<(&'a str, &'a str)> {
        if self.idx < self.params.len() {
            let idx = self.idx;
            let res = match self.params.segments[idx].1 {
                PathItem::Static(ref s) => &s,
                PathItem::Segment(s, e) => &self.params.path.path()[(s as usize)..(e as usize)],
            };
            self.idx += 1;
            return Some((&self.params.segments[idx].0, res));
        }
        None
    }
}

impl<'a, T: ResourcePath> Index<&'a str> for Path<T> {
    type Output = str;

    fn index(&self, name: &'a str) -> &str {
        self.get(name)
            .expect("Value for parameter is not available")
    }
}

impl<T: ResourcePath> Index<usize> for Path<T> {
    type Output = str;

    fn index(&self, idx: usize) -> &str {
        match self.segments[idx].1 {
            PathItem::Static(ref s) => &s,
            PathItem::Segment(s, e) => &self.path.path()[(s as usize)..(e as usize)],
        }
    }
}

impl<T: ResourcePath> Resource<T> for Path<T> {
    fn resource_path(&mut self) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "http")]
    #[test]
    fn test_get_param_by_name() {
        use crate::Url;
        use http::{HttpTryFrom, Uri};

        let mut params = Path::new(Url::new(Uri::try_from("/").unwrap()));
        params.add_static("item1", "path");
        params.add_static("item2", "http%3A%2F%2Flocalhost%3A80%2Ffoo");

        assert_eq!(params.get("item0"), None);
        assert_eq!(params.get_decoded("item0"), None);
        assert_eq!(params.get("item1"), Some("path"));
        assert_eq!(params.get_decoded("item1").unwrap().to_owned(), "path");
        assert_eq!(
            params.get("item2"),
            Some("http%3A%2F%2Flocalhost%3A80%2Ffoo")
        );
        assert_eq!(
            params.get_decoded("item2").unwrap().to_owned(),
            "http://localhost:80/foo"
        );
    }
}
