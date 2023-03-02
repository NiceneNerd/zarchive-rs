//! Handles reading and extracting from a ZArchive file.
//!
//! Basic usage is as follows:
//! ```rust
//! use zarchive::reader::ZArchiveReader;
//!
//! let reader = ZArchiveReader::open("test/crafting.zar").expect("Failed to read archive");
//! let file_data = reader.read_file("content/Model/Item_Feather.sbfres").expect("File not found");
//! assert_eq!(file_data.len(), 66416);
//! for entry in reader.iter().unwrap() {
//!    println!("{}", entry.name());
//! }
//! ```
use crate::{Result, ZArchiveError};
use cxx::{type_id, ExternType};
use std::{io::Write, path::Path, sync::RwLock};
use tinyvec::{array_vec, ArrayVec};

/// Wraps a handle to a file or directory node in an open archive.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash)]
#[repr(transparent)]
pub struct ZArchiveNodeHandle(u32);
const ZARCHIVE_INVALID_NODE: ZArchiveNodeHandle = ZArchiveNodeHandle(0xFFFFFFFF);

unsafe impl ExternType for ZArchiveNodeHandle {
    type Id = type_id!("ZArchiveNodeHandle");
    type Kind = cxx::kind::Trivial;
}

/// Represents an entry when iterating an archive directory, either a file or
/// subdirectory.
#[derive(Debug, Clone)]
pub struct DirEntry<'a> {
    inner: ffi::DirEntry<'a>,
    parent: ArrayVec<[&'a str; 5]>,
}

impl<'a> DirEntry<'a> {
    /// Returns the name of the entry.
    pub fn name(&self) -> &str {
        self.inner.name
    }

    /// Returns true if the entry is a file.
    pub fn is_file(&self) -> bool {
        self.inner.isFile
    }

    /// Returns true if the entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.inner.isDirectory
    }

    /// Returns the size of the entry, if it is a file.
    pub fn size(&self) -> Option<usize> {
        self.inner.isFile.then_some(self.inner.size as usize)
    }

    /// Returns the full path to the entry.
    pub fn full_path(&self) -> String {
        if self.parent.is_empty() {
            self.name().to_owned()
        } else {
            self.parent
                .iter()
                .chain([self.name()].iter())
                .map(|s| &**s)
                .collect::<Vec<&str>>()
                .join("/")
        }
    }

    /// Iterate over the directory contents, if the entry is a directory.
    pub fn iter<'reader: 'a>(
        &'a self,
        archive: &'reader ZArchiveReader,
    ) -> Option<ArchiveDirIterator<'a>> {
        archive.iter_dir(self).ok()
    }

    /// Count the directory contents, if the entry is a directory.
    pub fn count(&self, archive: &ZArchiveReader) -> Option<usize> {
        self.inner
            .isDirectory
            .then(|| archive.count_dir_entries(self).ok())
            .flatten()
    }
}

/// Iterator over the contents of a directory in an archive.
#[derive(Debug)]
pub struct ArchiveDirIterator<'a> {
    index: u32,
    count: u32,
    handle: ZArchiveNodeHandle,
    parent: ArrayVec<[&'a str; 5]>,
    reader: &'a ZArchiveReader,
    entry: ffi::DirEntry<'a>,
    started: bool,
}

impl<'a> ArchiveDirIterator<'a> {
    fn new(
        handle: ZArchiveNodeHandle,
        parent: ArrayVec<[&'a str; 5]>,
        reader: &'a ZArchiveReader,
    ) -> ArchiveDirIterator<'a> {
        ArchiveDirIterator {
            index: 0,
            count: 0,
            handle,
            parent,
            reader,
            entry: Default::default(),
            started: false,
        }
    }
}

impl<'a> Iterator for ArchiveDirIterator<'a> {
    type Item = DirEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            self.count = self
                .reader
                .0
                .read()
                .unwrap()
                .GetDirEntryCount(self.handle)
                .ok()?;
        }
        if self.index >= self.count {
            return None;
        }
        if self
            .reader
            .0
            .read()
            .unwrap()
            .GetDirEntry(self.handle, self.index, &mut self.entry)
            .ok()?
        {
            self.index += 1;
            Some(DirEntry {
                inner: self.entry.clone(),
                parent: self.parent,
            })
        } else {
            None
        }
    }
}

/// Represents an open ZArchive, wrapping the C++ type.  
///
/// It holds an open file handle to the archive on disk, which it retains until
/// destroyed. The archive is read-only, but the C++ struct mutates constantly
/// for many operations. For this reason, the Rust struct wraps it in an
/// [`RwLock`](std::sync::RwLock) to provide a simple immutable interface that
/// works as expected in any context, including mulithreaded.
pub struct ZArchiveReader(RwLock<cxx::UniquePtr<ffi::ZArchiveReader>>);

impl std::fmt::Debug for ZArchiveReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZArchiveReader")
    }
}

unsafe impl Send for ZArchiveReader {}
unsafe impl Sync for ZArchiveReader {}

impl ZArchiveReader {
    /// Open a ZArchive from a file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self(RwLock::new(ffi::OpenFromFile(
            path.as_ref().to_str().ok_or_else(|| {
                ZArchiveError::InvalidFilePath(path.as_ref().to_string_lossy().to_string())
            })?,
        )?)))
    }

    /// Get the size of a file in the archive, if the file exists.
    pub fn file_size(&self, file: impl AsRef<Path>) -> Option<usize> {
        let file = file.as_ref().to_str()?;
        let mut archive = self.0.write().unwrap();
        let node_handle = archive.pin_mut().LookUp(file, true, false).ok()?;
        archive
            .pin_mut()
            .GetFileSize(node_handle)
            .ok()
            .map(|s| s as usize)
    }

    /// Read a file from the archive into a `Vec<u8>`, if the file exists.
    pub fn read_file(&self, file: impl AsRef<Path>) -> Option<Vec<u8>> {
        let mut reader = self.0.write().unwrap();
        let handle = reader
            .pin_mut()
            .LookUp(file.as_ref().to_str()?, true, false)
            .ok()?;
        if handle == ZARCHIVE_INVALID_NODE {
            None
        } else {
            let size = reader.pin_mut().GetFileSize(handle).ok()?;
            let mut buffer: Vec<u8> = Vec::with_capacity(size as usize);
            unsafe {
                let written = reader
                    .pin_mut()
                    .ReadFromFile(handle, 0, size, buffer.as_mut_ptr())
                    .unwrap();
                if written != size {
                    panic!(
                        "Wrote an unexpected number of bytes, expected {} but got {}",
                        size, written
                    );
                }
                buffer.set_len(written as usize);
            };
            Some(buffer)
        }
    }

    /// Extract a file from the archive to disk, if the file exists. If the destination
    /// is an existing directory, the file will be extracted into the directory with its
    /// relative path in the archive. Otherwise it will be extracted to the destination
    /// path as-is.
    pub fn extract_file(&self, file: impl AsRef<Path>, dest: impl AsRef<Path>) -> Result<()> {
        let file = file.as_ref().to_str().ok_or_else(|| {
            ZArchiveError::InvalidFilePath(file.as_ref().to_string_lossy().to_string())
        })?;
        let dest = if dest.as_ref().is_dir() {
            dest.as_ref().join(file)
        } else {
            dest.as_ref().to_path_buf()
        };
        dest.parent().map(std::fs::create_dir_all).transpose()?;
        let handle = self
            .0
            .write()
            .unwrap()
            .pin_mut()
            .LookUp(file, true, false)?;
        if handle == ZARCHIVE_INVALID_NODE || !self.0.read().unwrap().IsFile(handle)? {
            Err(ZArchiveError::MissingFile(file.to_owned()))
        } else {
            let mut reader = self.0.write().unwrap();
            let size = reader.pin_mut().GetFileSize(handle)?;
            let mut dest_handle = std::fs::File::create(dest)?;
            dest_handle.set_len(size)?;
            let mut buffer = vec![0; size as usize];
            unsafe {
                let written = reader
                    .pin_mut()
                    .ReadFromFile(handle, 0, size, buffer.as_mut_ptr())
                    .unwrap();
                if written != size {
                    panic!(
                        "Wrote an unexpected number of bytes, expected {} but got {}",
                        size, written
                    );
                }
                buffer.set_len(written as usize);
            };
            std::io::BufWriter::new(&mut dest_handle).write_all(&buffer)?;
            Ok(())
        }
    }

    /// Extract the entire archive to disk.
    pub fn extract(&self, dest: impl AsRef<Path>) -> Result<()> {
        let dest = dest.as_ref();
        if dest.is_file() {
            Err(ZArchiveError::InvalidDestination(
                dest.to_string_lossy().to_string(),
            ))
        } else {
            self.get_files().unwrap().into_iter().try_for_each(|file| {
                let dest = dest.join(&file);
                if !dest.parent().unwrap().exists() {
                    std::fs::create_dir_all(dest.parent().unwrap())?;
                }
                self.extract_file(&file, &dest)
            })
        }
    }

    /// Read part of a file from the archive into a `Vec<u8>` using the specified
    /// length and offet, if the file exists.
    pub fn read_from_file(
        &self,
        file: impl AsRef<Path>,
        offset: usize,
        length: usize,
    ) -> Option<Vec<u8>> {
        let mut reader = self.0.write().unwrap();
        let handle = reader
            .pin_mut()
            .LookUp(file.as_ref().to_str()?, true, false)
            .ok()?;
        if handle == ZARCHIVE_INVALID_NODE {
            None
        } else {
            let size = reader.pin_mut().GetFileSize(handle).ok()?;
            if length > size as usize {
                return None;
            }
            let mut buffer: Vec<u8> = Vec::with_capacity(length);
            unsafe {
                let written = reader
                    .pin_mut()
                    .ReadFromFile(handle, offset as u64, length as u64, buffer.as_mut_ptr())
                    .unwrap();
                if written != length as u64 {
                    panic!(
                        "Wrote an unexpected number of bytes, expected {} but got {}",
                        length, written
                    );
                }
                buffer.set_len(written as usize);
            };
            Some(buffer)
        }
    }

    /// Get a list of all the files in the archive (more convenient than manual
    /// iteration if you can spare the allocation).
    pub fn get_files(&self) -> Result<Vec<String>> {
        fn process_dir_entry(
            archive: &ZArchiveReader,
            files: &mut Vec<String>,
            node_handle: ZArchiveNodeHandle,
            parent: &str,
            dir_entry: &mut ffi::DirEntry,
        ) -> Result<()> {
            let count = archive.0.read().unwrap().GetDirEntryCount(node_handle)?;
            for i in 0..count {
                if archive
                    .0
                    .read()
                    .unwrap()
                    .GetDirEntry(node_handle, i, dir_entry)?
                {
                    let full_path = if !parent.is_empty() {
                        [parent, dir_entry.name].join("/")
                    } else {
                        dir_entry.name.to_owned()
                    };
                    if dir_entry.isFile {
                        files.push(full_path);
                    } else if dir_entry.isDirectory {
                        let next = archive
                            .0
                            .write()
                            .unwrap()
                            .pin_mut()
                            .LookUp(&full_path, false, true)?;
                        if next != ZARCHIVE_INVALID_NODE {
                            process_dir_entry(archive, files, next, &full_path, dir_entry)?;
                        }
                    }
                }
            }
            Ok(())
        }

        let mut dir_entry = ffi::DirEntry::default();
        let mut files = vec![];
        let root = self.0.write().unwrap().pin_mut().LookUp("", false, true)?;
        if root != ZARCHIVE_INVALID_NODE {
            process_dir_entry(self, &mut files, root, "", &mut dir_entry)?;
        }
        Ok(files)
    }

    /// Iterate over the contents of the root directory of the archive.
    pub fn iter(&self) -> Result<ArchiveDirIterator<'_>> {
        let root = self.0.write().unwrap().pin_mut().LookUp("", false, true)?;
        if root == ZARCHIVE_INVALID_NODE {
            Err(ZArchiveError::MissingFile("archive root".to_owned()))
        } else {
            Ok(ArchiveDirIterator::new(root, array_vec![], self))
        }
    }

    /// Iterate over the contents of a directory in the archive.
    pub fn iter_dir<'a, 'entry>(
        &'a self,
        dir: &'entry DirEntry<'a>,
    ) -> Result<ArchiveDirIterator<'entry>>
    where
        'a: 'entry,
    {
        let node_handle =
            self.0
                .write()
                .unwrap()
                .pin_mut()
                .LookUp(&dir.full_path(), false, true)?;
        if node_handle == ZARCHIVE_INVALID_NODE {
            Err(ZArchiveError::MissingFile(dir.full_path()))
        } else if !dir.is_dir() {
            Err(ZArchiveError::NotADirectory(dir.full_path()))
        } else {
            Ok(ArchiveDirIterator::new(
                node_handle,
                dir.parent
                    .iter()
                    .copied()
                    .chain([dir.name()].into_iter())
                    .collect(),
                self,
            ))
        }
    }

    /// Count the contents of a directory in the archive.
    pub fn count_dir_entries<'a>(&'a self, dir: &'a DirEntry) -> Result<usize> {
        let mut reader = self.0.write().unwrap();
        let node_handle = reader.pin_mut().LookUp(&dir.full_path(), false, true)?;
        if node_handle == ZARCHIVE_INVALID_NODE {
            Err(ZArchiveError::MissingFile(dir.full_path()))
        } else if !dir.is_dir() {
            Err(ZArchiveError::NotADirectory(dir.full_path()))
        } else {
            Ok(reader.pin_mut().GetDirEntryCount(node_handle)? as usize)
        }
    }
}

#[cxx::bridge]
mod ffi {
    #[derive(Debug, Default, Clone)]
    #[allow(non_snake_case)]
    struct DirEntry<'a> {
        name: &'a str,
        isFile: bool,
        isDirectory: bool,
        size: u64,
    }

    unsafe extern "C++" {
        include!("zarchive/include/zarchive/zarchivereader.h");

        type ZArchiveNodeHandle = super::ZArchiveNodeHandle;
        type ZArchiveReader;
        fn OpenFromFile(path: &str) -> Result<UniquePtr<ZArchiveReader>>;
        fn LookUp(
            self: Pin<&mut ZArchiveReader>,
            path: &str,
            allowFile: bool,
            allowDirectory: bool,
        ) -> Result<ZArchiveNodeHandle>;
        #[allow(unused)]
        fn IsDirectory(self: &ZArchiveReader, nodeHandle: ZArchiveNodeHandle) -> Result<bool>;
        fn IsFile(self: &ZArchiveReader, nodeHandle: ZArchiveNodeHandle) -> Result<bool>;
        fn GetDirEntryCount(self: &ZArchiveReader, nodeHandle: ZArchiveNodeHandle) -> Result<u32>;
        fn GetDirEntry<'a>(
            self: &'a ZArchiveReader,
            nodeHandle: ZArchiveNodeHandle,
            index: u32,
            dirEntry: &'a mut DirEntry,
        ) -> Result<bool>;
        fn GetFileSize(
            self: Pin<&mut ZArchiveReader>,
            nodeHandle: ZArchiveNodeHandle,
        ) -> Result<u64>;
        unsafe fn ReadFromFile(
            self: Pin<&mut ZArchiveReader>,
            nodeHandle: ZArchiveNodeHandle,
            offset: u64,
            size: u64,
            buffer: *mut u8,
        ) -> Result<u64>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_list() {
        let archive = ZArchiveReader::open("test/crafting.zar").unwrap();
        for file in archive.get_files().unwrap() {
            println!("{}", file);
        }
    }

    #[test]
    fn walk_tree() {
        let archive = ZArchiveReader::open("test/crafting.zar").unwrap();
        fn print_dir<'a, 'b>(archive: &'a ZArchiveReader, dir: &'b DirEntry<'a>)
        where
            'a: 'b,
        {
            for entry in archive.iter_dir(dir).unwrap() {
                if entry.is_file() {
                    println!("{}", entry.full_path());
                } else {
                    print_dir(archive, &entry);
                }
            }
        }

        for entry in archive.iter().unwrap() {
            if entry.is_file() {
                println!("{}", entry.full_path());
            } else {
                print_dir(&archive, &entry);
            }
        }
    }

    #[test]
    fn extract_file() {
        let archive = ZArchiveReader::open("test/crafting.zar").unwrap();
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        archive
            .extract_file("content/Actor/ActorInfo.product.sbyml", temp_file.path())
            .unwrap();
    }

    #[test]
    fn extract_all() {
        let temp_dir = tempfile::tempdir().unwrap();
        let archive = ZArchiveReader::open("test/crafting.zar").unwrap();
        let files = archive.get_files().unwrap();
        archive.extract(temp_dir.path()).unwrap();
        for file in files {
            assert!(temp_dir.path().join(file).exists());
        }
    }

    #[test]
    fn partial_read() {
        let archive = ZArchiveReader::open("test/crafting.zar").unwrap();
        let data = archive
            .read_from_file("content/Pack/Bootup.pack", 0, 4)
            .unwrap();
        assert_eq!(&data[..4], b"SARC");
    }

    #[test]
    fn concurrency() {
        use rayon::prelude::*;

        let archive = ZArchiveReader::open("test/crafting.zar").unwrap();
        let files = archive.get_files().unwrap();
        files.into_par_iter().for_each(|file| {
            if let Some(data) = archive.read_from_file(&file, 0, 4) {
                println!("{}", std::str::from_utf8(&data[..4]).unwrap());
            } else if !file.contains("AocMainField") {
                panic!("Failed to get data for {}", file);
            }
        });
    }

    #[test]
    fn ffi_methods() {
        let mut archive: cxx::UniquePtr<ffi::ZArchiveReader> =
            ffi::OpenFromFile("test/crafting.zar").unwrap();
        println!("Opened archive");
        let file_handle = archive
            .pin_mut()
            .LookUp("content/Pack/Bootup.pack", true, false)
            .unwrap();
        println!("Did we find it? {:?}", file_handle != ZARCHIVE_INVALID_NODE);
        println!("Is it a file? {}", archive.IsFile(file_handle).unwrap());
        println!(
            "Is it a directory? {}",
            archive.IsDirectory(file_handle).unwrap()
        );
        let size = archive.pin_mut().GetFileSize(file_handle).unwrap();
        println!("What size is it? {:.2} MB", (size as f64 / 1024.0 / 1024.0));
        let mut buffer: Vec<u8> = Vec::with_capacity(size as usize);
        let written = unsafe {
            let written = archive
                .pin_mut()
                .ReadFromFile(file_handle, 0, size, buffer.as_mut_ptr())
                .unwrap();
            buffer.set_len(written as usize);
            written
        };
        assert_eq!(written, size);
        assert_eq!(&buffer[..4], b"SARC");
        println!("First file is good, let's check the others");
        let mut dir_entry = ffi::DirEntry::default();
        let root = archive.pin_mut().LookUp("", true, true).unwrap();
        assert_ne!(root, ZARCHIVE_INVALID_NODE);

        fn print_dir_entry(
            node_handle: ZArchiveNodeHandle,
            parent: &str,
            archive: &mut cxx::UniquePtr<ffi::ZArchiveReader>,
            dir_entry: &mut ffi::DirEntry,
        ) {
            let count = archive.GetDirEntryCount(node_handle).unwrap();
            for i in 0..count {
                if archive.GetDirEntry(node_handle, i, dir_entry).unwrap() {
                    let full_path = if !parent.is_empty() {
                        format!("{}/{}", parent, dir_entry.name)
                    } else {
                        dir_entry.name.to_string()
                    };
                    if dir_entry.isFile {
                        println!("{}", &full_path);
                    } else if dir_entry.isDirectory {
                        let next = archive.pin_mut().LookUp(&full_path, false, true).unwrap();
                        assert_ne!(next, ZARCHIVE_INVALID_NODE, "{}", &full_path);
                        print_dir_entry(next, &full_path, archive, dir_entry);
                    }
                }
            }
        }

        print_dir_entry(root, "", &mut archive, &mut dir_entry);
    }
}
