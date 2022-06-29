use cxx::{type_id, ExternType};
use smallvec::SmallVec;
use std::{cell::RefCell, io::Write, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZArchiveError {
    #[error("Invalid file path: {0}")]
    InvalidFilePath(String),
    #[error("Archive entry is not a directory: {0}")]
    NotADirectory(String),
    #[error("File not in archive: {0}")]
    MissingFile(String),
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("{0}")]
    Other(#[from] cxx::Exception),
}
type Result<T> = std::result::Result<T, ZArchiveError>;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash)]
#[repr(transparent)]
pub struct ZArchiveNodeHandle(u32);
const ZARCHIVE_INVALID_NODE: ZArchiveNodeHandle = ZArchiveNodeHandle(0xFFFFFFFF);

unsafe impl ExternType for ZArchiveNodeHandle {
    type Id = type_id!("ZArchiveNodeHandle");
    type Kind = cxx::kind::Trivial;
}

#[derive(Debug, Clone)]
pub struct DirEntry<'a> {
    inner: ffi::DirEntry<'a>,
    parent: SmallVec<[&'a str; 5]>,
}

impl<'a> DirEntry<'a> {
    pub fn name(&self) -> &str {
        self.inner.name
    }

    pub fn is_file(&self) -> bool {
        self.inner.isFile
    }

    pub fn is_dir(&self) -> bool {
        self.inner.isDirectory
    }

    pub fn size(&self) -> Option<usize> {
        self.inner.isFile.then(|| self.inner.size as usize)
    }

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

    pub fn iter<'b>(&'a self, archive: &'b ZArchiveReader) -> Option<ArchiveDirIterator<'a>>
    where
        'b: 'a,
    {
        archive.iter_dir(self).ok()
    }

    pub fn count(&self, archive: &ZArchiveReader) -> Option<usize> {
        self.inner
            .isDirectory
            .then(|| archive.count_dir_entries(self).ok())
            .flatten()
    }
}

#[derive(Debug)]
pub struct ArchiveDirIterator<'a> {
    index: u32,
    count: u32,
    handle: ZArchiveNodeHandle,
    parent: SmallVec<[&'a str; 5]>,
    reader: &'a ZArchiveReader,
    entry: ffi::DirEntry<'a>,
    started: bool,
}

impl<'a> ArchiveDirIterator<'a> {
    pub fn new(
        handle: ZArchiveNodeHandle,
        parent: SmallVec<[&'a str; 5]>,
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
            self.count = self.reader.0.borrow().GetDirEntryCount(self.handle).ok()?;
        }
        if self.index >= self.count {
            return None;
        }
        if self
            .reader
            .0
            .borrow()
            .GetDirEntry(self.handle, self.index, &mut self.entry)
            .ok()?
        {
            self.index += 1;
            Some(DirEntry {
                inner: self.entry.clone(),
                parent: self.parent.clone(),
            })
        } else {
            None
        }
    }
}

pub struct ZArchiveReader(std::cell::RefCell<cxx::UniquePtr<ffi::ZArchiveReader>>);

impl std::fmt::Debug for ZArchiveReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZArchiveReader")
    }
}

unsafe impl Send for ZArchiveReader {}
unsafe impl Sync for ZArchiveReader {}

impl ZArchiveReader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self(RefCell::new(ffi::OpenFromFile(
            path.as_ref().to_str().ok_or_else(|| {
                ZArchiveError::InvalidFilePath(path.as_ref().to_string_lossy().to_string())
            })?,
        )?)))
    }

    pub fn read_file(&self, file: impl AsRef<Path>) -> Option<Vec<u8>> {
        let handle = self
            .0
            .borrow_mut()
            .pin_mut()
            .LookUp(file.as_ref().to_str()?, true, false)
            .ok()?;
        if handle == ZARCHIVE_INVALID_NODE {
            None
        } else {
            let size = self.0.borrow_mut().pin_mut().GetFileSize(handle).ok()?;
            let mut buffer: Vec<u8> = Vec::with_capacity(size as usize);
            unsafe {
                let written = self
                    .0
                    .borrow_mut()
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
        let handle = self.0.borrow_mut().pin_mut().LookUp(file, true, false)?;
        if handle == ZARCHIVE_INVALID_NODE || !self.0.borrow().IsFile(handle)? {
            Err(ZArchiveError::MissingFile(file.to_owned()))
        } else {
            let size = self.0.borrow_mut().pin_mut().GetFileSize(handle)?;
            let mut dest_handle = std::fs::File::create(dest)?;
            dest_handle.set_len(size as u64)?;
            let mut buffer = vec![0; size as usize];
            unsafe {
                let written = self
                    .0
                    .borrow_mut()
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

    pub fn read_from_file(
        &self,
        file: impl AsRef<Path>,
        offset: usize,
        length: usize,
    ) -> Option<Vec<u8>> {
        let handle = self
            .0
            .borrow_mut()
            .pin_mut()
            .LookUp(file.as_ref().to_str()?, true, false)
            .ok()?;
        if handle == ZARCHIVE_INVALID_NODE {
            None
        } else {
            let size = self.0.borrow_mut().pin_mut().GetFileSize(handle).ok()?;
            if length > size as usize {
                return None;
            }
            let mut buffer: Vec<u8> = Vec::with_capacity(length);
            unsafe {
                let written = self
                    .0
                    .borrow_mut()
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

    pub fn get_files(&self) -> Result<Vec<String>> {
        fn process_dir_entry(
            archive: &ZArchiveReader,
            files: &mut Vec<String>,
            node_handle: ZArchiveNodeHandle,
            parent: &str,
            dir_entry: &mut ffi::DirEntry,
        ) -> Result<()> {
            let count = archive.0.borrow().GetDirEntryCount(node_handle)?;
            for i in 0..count {
                if archive.0.borrow().GetDirEntry(node_handle, i, dir_entry)? {
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
                            .borrow_mut()
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
        let root = self.0.borrow_mut().pin_mut().LookUp("", false, true)?;
        if root != ZARCHIVE_INVALID_NODE {
            process_dir_entry(self, &mut files, root, "", &mut dir_entry)?;
        }
        Ok(files)
    }

    pub fn iter(&self) -> Result<ArchiveDirIterator<'_>> {
        let root = self.0.borrow_mut().pin_mut().LookUp("", false, true)?;
        if root == ZARCHIVE_INVALID_NODE {
            Err(ZArchiveError::MissingFile("archive root".to_owned()))
        } else {
            Ok(ArchiveDirIterator::new(root, smallvec::smallvec![], self))
        }
    }

    pub fn iter_dir<'a, 'b>(&'a self, dir: &'b DirEntry<'a>) -> Result<ArchiveDirIterator<'b>>
    where
        'a: 'b,
    {
        let node_handle = self
            .0
            .borrow_mut()
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

    pub fn count_dir_entries<'a>(&'a self, dir: &'a DirEntry) -> Result<usize> {
        let node_handle = self
            .0
            .borrow_mut()
            .pin_mut()
            .LookUp(&dir.full_path(), false, true)?;
        if node_handle == ZARCHIVE_INVALID_NODE {
            Err(ZArchiveError::MissingFile(dir.full_path()))
        } else if !dir.is_dir() {
            Err(ZArchiveError::NotADirectory(dir.full_path()))
        } else {
            Ok(self
                .0
                .borrow_mut()
                .pin_mut()
                .GetDirEntryCount(node_handle)? as usize)
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

        type ZArchiveNodeHandle = crate::ZArchiveNodeHandle;
        type ZArchiveReader;
        fn OpenFromFile(path: &str) -> Result<UniquePtr<ZArchiveReader>>;
        fn LookUp(
            self: Pin<&mut ZArchiveReader>,
            path: &str,
            allowFile: bool,
            allowDirectory: bool,
        ) -> Result<ZArchiveNodeHandle>;
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
    use rayon::iter::{IntoParallelIterator, ParallelIterator};

    #[test]
    fn file_list() {
        let archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        for file in archive.get_files().unwrap() {
            println!("{}", file);
        }
    }

    #[test]
    fn walk_tree() {
        let archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        fn print_dir<'a, 'b>(archive: &'a ZArchiveReader, dir: &'b DirEntry<'a>)
        where
            'a: 'b,
        {
            for entry in archive.iter_dir(dir).unwrap() {
                if entry.is_file() {
                    println!("{}", entry.full_path());
                } else {
                    print_dir(&archive, &entry);
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
        let archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        archive
            .extract_file(
                "content/Actor/ActorInfo.product.sbyml",
                "test/ActorInfo.product.sbyml",
            )
            .unwrap();
    }

    #[test]
    fn partial_read() {
        let archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        let data = archive
            .read_from_file("content/Pack/Bootup.pack", 0, 4)
            .unwrap();
        assert_eq!(&data[..4], b"SARC");
    }

    #[test]
    fn concurrency() {
        use std::sync::{Arc, Mutex};
        let archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        let files = archive.get_files().unwrap();
        let archive = Arc::new(Mutex::new(archive));
        files.into_par_iter().for_each(|file| {
            if let Some(data) = archive.lock().unwrap().read_from_file(&file, 0, 4) {
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
