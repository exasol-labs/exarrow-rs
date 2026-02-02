//! File source types for parallel import operations.
//!
//! This module provides the `IntoFileSources` trait which allows both single file paths
//! and collections of file paths to be used with the parallel import functions.

use std::path::{Path, PathBuf};

/// Trait for types that can be converted to a collection of file sources.
///
/// This trait enables the parallel import functions to accept both single paths
/// and collections of paths, providing a flexible API.
///
/// # Example
///
/// ```rust
/// use std::path::PathBuf;
/// use exarrow_rs::import::IntoFileSources;
///
/// // Single path
/// let single = PathBuf::from("/data/file.csv");
/// let sources: Vec<PathBuf> = single.into_sources();
/// assert_eq!(sources.len(), 1);
///
/// // Multiple paths
/// let multiple = vec![
///     PathBuf::from("/data/file1.csv"),
///     PathBuf::from("/data/file2.csv"),
/// ];
/// let sources: Vec<PathBuf> = multiple.into_sources();
/// assert_eq!(sources.len(), 2);
/// ```
pub trait IntoFileSources {
    /// Convert this type into a vector of file paths.
    fn into_sources(self) -> Vec<PathBuf>;
}

impl IntoFileSources for PathBuf {
    fn into_sources(self) -> Vec<PathBuf> {
        vec![self]
    }
}

impl IntoFileSources for &Path {
    fn into_sources(self) -> Vec<PathBuf> {
        vec![self.to_path_buf()]
    }
}

impl IntoFileSources for &PathBuf {
    fn into_sources(self) -> Vec<PathBuf> {
        vec![self.clone()]
    }
}

impl IntoFileSources for Vec<PathBuf> {
    fn into_sources(self) -> Vec<PathBuf> {
        self
    }
}

impl IntoFileSources for &[PathBuf] {
    fn into_sources(self) -> Vec<PathBuf> {
        self.to_vec()
    }
}

impl<const N: usize> IntoFileSources for [PathBuf; N] {
    fn into_sources(self) -> Vec<PathBuf> {
        self.into_iter().collect()
    }
}

impl<const N: usize> IntoFileSources for &[PathBuf; N] {
    fn into_sources(self) -> Vec<PathBuf> {
        self.iter().cloned().collect()
    }
}

// Also support String and &str for convenience
impl IntoFileSources for String {
    fn into_sources(self) -> Vec<PathBuf> {
        vec![PathBuf::from(self)]
    }
}

impl IntoFileSources for &str {
    fn into_sources(self) -> Vec<PathBuf> {
        vec![PathBuf::from(self)]
    }
}

impl IntoFileSources for Vec<String> {
    fn into_sources(self) -> Vec<PathBuf> {
        self.into_iter().map(PathBuf::from).collect()
    }
}

impl IntoFileSources for Vec<&str> {
    fn into_sources(self) -> Vec<PathBuf> {
        self.into_iter().map(PathBuf::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pathbuf_into_sources() {
        let path = PathBuf::from("/data/file.csv");
        let sources = path.into_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0], PathBuf::from("/data/file.csv"));
    }

    #[test]
    fn test_path_ref_into_sources() {
        let path = Path::new("/data/file.csv");
        let sources = path.into_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0], PathBuf::from("/data/file.csv"));
    }

    #[test]
    fn test_pathbuf_ref_into_sources() {
        let path = PathBuf::from("/data/file.csv");
        let sources = (&path).into_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0], PathBuf::from("/data/file.csv"));
    }

    #[test]
    fn test_vec_pathbuf_into_sources() {
        let paths = vec![
            PathBuf::from("/data/file1.csv"),
            PathBuf::from("/data/file2.csv"),
            PathBuf::from("/data/file3.csv"),
        ];
        let sources = paths.into_sources();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0], PathBuf::from("/data/file1.csv"));
        assert_eq!(sources[1], PathBuf::from("/data/file2.csv"));
        assert_eq!(sources[2], PathBuf::from("/data/file3.csv"));
    }

    #[test]
    fn test_slice_pathbuf_into_sources() {
        let paths = vec![
            PathBuf::from("/data/file1.csv"),
            PathBuf::from("/data/file2.csv"),
        ];
        let sources = paths.as_slice().into_sources();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn test_array_pathbuf_into_sources() {
        let paths: [PathBuf; 2] = [
            PathBuf::from("/data/file1.csv"),
            PathBuf::from("/data/file2.csv"),
        ];
        let sources = paths.into_sources();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn test_array_ref_pathbuf_into_sources() {
        let paths: [PathBuf; 2] = [
            PathBuf::from("/data/file1.csv"),
            PathBuf::from("/data/file2.csv"),
        ];
        let sources = (&paths).into_sources();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn test_string_into_sources() {
        let path = String::from("/data/file.csv");
        let sources = path.into_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0], PathBuf::from("/data/file.csv"));
    }

    #[test]
    fn test_str_into_sources() {
        let sources = "/data/file.csv".into_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0], PathBuf::from("/data/file.csv"));
    }

    #[test]
    fn test_vec_string_into_sources() {
        let paths = vec![
            String::from("/data/file1.csv"),
            String::from("/data/file2.csv"),
        ];
        let sources = paths.into_sources();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn test_vec_str_into_sources() {
        let paths = vec!["/data/file1.csv", "/data/file2.csv"];
        let sources = paths.into_sources();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn test_empty_vec_into_sources() {
        let paths: Vec<PathBuf> = vec![];
        let sources = paths.into_sources();
        assert!(sources.is_empty());
    }
}
