use cxx::{type_id, ExternType};
use std::{io::Write, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZArchiveError {
    #[error("Invalid file path: {0}")]
    InvalidFilePath(String),
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

pub struct ArchiveIterator<'a> {
    reader: &'a mut ZArchiveReader,
    files: Vec<String>,
    index: usize,
    filled: bool,
}

impl<'a> ArchiveIterator<'a> {
    pub fn new(reader: &'a mut ZArchiveReader) -> Self {
        Self {
            reader,
            files: Vec::new(),
            index: 0,
            filled: false,
        }
    }

    pub fn into_vec(self) -> Vec<String> {
        self.files
    }
}

impl<'a> ArchiveIterator<'a> {
    fn process_dir_entry(
        &mut self,
        node_handle: ZArchiveNodeHandle,
        parent: &str,
        dir_entry: &mut ffi::DirEntry,
    ) -> Result<()> {
        let count = self.reader.0.GetDirEntryCount(node_handle)?;
        for i in 0..count {
            if self.reader.0.GetDirEntry(node_handle, i, dir_entry)? {
                let full_path = if !parent.is_empty() {
                    [parent, dir_entry.name].join("/")
                } else {
                    dir_entry.name.to_owned()
                };
                if dir_entry.isFile {
                    self.files.push(full_path);
                } else if dir_entry.isDirectory {
                    let next = self.reader.0.pin_mut().LookUp(&full_path, false, true)?;
                    if next != ZARCHIVE_INVALID_NODE {
                        self.process_dir_entry(next, &full_path, dir_entry)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<'a> Iterator for ArchiveIterator<'a> {
    type Item = String;
    fn next(&mut self) -> Option<Self::Item> {
        if !self.filled {
            let mut dir_entry = ffi::DirEntry::default();
            let root = self.reader.0.pin_mut().LookUp("", true, true).unwrap();
            if root == ZARCHIVE_INVALID_NODE {
                return None;
            }
            self.process_dir_entry(root, "", &mut dir_entry).ok()?;
            self.filled = true;
        }
        if self.index < self.files.len() {
            let result = self.files.swap_remove(self.index);
            self.index += 1;
            Some(result)
        } else {
            None
        }
    }
}

pub struct ZArchiveReader(cxx::UniquePtr<ffi::ZArchiveReader>);

impl ZArchiveReader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self(ffi::OpenFromFile(
            path.as_ref().to_str().ok_or_else(|| {
                ZArchiveError::InvalidFilePath(path.as_ref().to_string_lossy().to_string())
            })?,
        )?))
    }

    pub fn read_file(&mut self, file: impl AsRef<Path>) -> Option<Vec<u8>> {
        let handle = self
            .0
            .pin_mut()
            .LookUp(file.as_ref().to_str()?, true, false)
            .ok()?;
        if handle == ZARCHIVE_INVALID_NODE {
            None
        } else {
            let size = self.0.pin_mut().GetFileSize(handle).ok()?;
            let mut buffer: Vec<u8> = Vec::with_capacity(size as usize);
            unsafe {
                let written = self
                    .0
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

    pub fn extract_file(&mut self, file: impl AsRef<Path>, dest: impl AsRef<Path>) -> Result<()> {
        let file = file.as_ref().to_str().ok_or_else(|| {
            ZArchiveError::InvalidFilePath(file.as_ref().to_string_lossy().to_string())
        })?;
        let dest = if dest.as_ref().is_dir() {
            dest.as_ref().join(file)
        } else {
            dest.as_ref().to_path_buf()
        };
        dest.parent().map(std::fs::create_dir_all).transpose()?;
        let handle = self.0.pin_mut().LookUp(file, true, false)?;
        if handle == ZARCHIVE_INVALID_NODE || !self.0.IsFile(handle)? {
            Err(ZArchiveError::MissingFile(file.to_owned()))
        } else {
            let size = self.0.pin_mut().GetFileSize(handle)?;
            let mut dest_handle = std::fs::File::create(dest)?;
            dest_handle.set_len(size as u64)?;
            let mut buffer = vec![0; size as usize];
            unsafe {
                let written = self
                    .0
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
        &mut self,
        file: impl AsRef<Path>,
        offset: usize,
        length: usize,
    ) -> Option<Vec<u8>> {
        let handle = self
            .0
            .pin_mut()
            .LookUp(file.as_ref().to_str()?, true, false)
            .ok()?;
        if handle == ZARCHIVE_INVALID_NODE {
            None
        } else {
            let size = self.0.pin_mut().GetFileSize(handle).ok()?;
            if length > size as usize {
                return None;
            }
            let mut buffer: Vec<u8> = Vec::with_capacity(length);
            unsafe {
                let written = self
                    .0
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

    pub fn files(&mut self) -> ArchiveIterator<'_> {
        ArchiveIterator::new(self)
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

    #[test]
    fn iter_files() {
        let mut archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        for file in archive.files() {
            println!("{}", file);
        }
    }

    #[test]
    fn extract_file() {
        let mut archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        archive
            .extract_file(
                "content/Actor/ActorInfo.product.sbyml",
                "test/ActorInfo.product.sbyml",
            )
            .unwrap();
    }

    #[test]
    fn partial_read() {
        let mut archive = ZArchiveReader::new("test/crafting.zar").unwrap();
        let data = archive
            .read_from_file("content/Pack/Bootup.pack", 0, 4)
            .unwrap();
        assert_eq!(&data[..4], b"SARC");
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
