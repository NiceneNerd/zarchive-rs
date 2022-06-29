use thiserror::Error;

static ZARCHIVE_INVALID_NODE: u32 = 0xFFFFFFFF;

#[derive(Debug, Error)]
pub enum ZArchiveError {
    #[error("Invalid file path: {0}")]
    InvalidFilePath(String),
    #[error("{0}")]
    Other(#[from] cxx::Exception),
}

type Result<T> = std::result::Result<T, ZArchiveError>;

pub struct ArchiveIterator<'a> {
    reader: &'a mut ZArchiveReader,
    todo: Vec<(String, &'a str, u32, u32)>,
    index: u32,
    started: bool,
    dir_entry: ffi::DirEntry<'a>,
}

impl<'a> ArchiveIterator<'a> {
    pub fn new(reader: &'a mut ZArchiveReader) -> Self {
        Self {
            reader,
            todo: Vec::new(),
            index: 0,
            started: false,
            dir_entry: Default::default(),
        }
    }
}

impl<'a> Iterator for ArchiveIterator<'a> {
    type Item = String;
    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            let root = self.reader.0.pin_mut().LookUp("", false, true).ok()?;
            if root == ZARCHIVE_INVALID_NODE {
                return None;
            }
            self.todo.push(("".to_owned(), "", root, 0));
            self.started = true;
        }
        self.todo.pop().and_then(|(parent, name, handle, index)| {
            println!("Looking at {}, index {}", parent, index);
            if self.reader.0.IsFile(handle).ok()? {
                if parent.is_empty() {
                    Some(name.to_owned())
                } else {
                    Some([&parent, name].join("/"))
                }
            } else if self.reader.0.IsDirectory(handle).ok()? {
                let path = if !parent.is_empty() {
                    [&parent, name].join("/")
                } else {
                    name.to_owned()
                };
                println!("Checking {}", path);
                let next_handle = self.reader.0.pin_mut().LookUp(&path, false, true).ok()?;
                let count = self.reader.0.GetDirEntryCount(next_handle).ok()?;
                for i in 0..count {
                    if self
                        .reader
                        .0
                        .GetDirEntry(next_handle, i, &mut self.dir_entry)
                        .ok()?
                    {
                        let sub_path = if !path.is_empty() {
                            [&path, self.dir_entry.name].join("/")
                        } else {
                            self.dir_entry.name.to_owned()
                        };
                        let sub_handle =
                            self.reader.0.pin_mut().LookUp(&sub_path, true, true).ok()?;
                        println!("Adding {}", sub_path);
                        self.todo
                            .push((path.clone(), self.dir_entry.name, sub_handle, i));
                    }
                }
                self.next()
            } else {
                None
            }
        })
    }
}

pub struct ZArchiveReader(cxx::UniquePtr<ffi::ZArchiveReader>);

impl ZArchiveReader {
    pub fn new(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self(ffi::OpenFromFile(
            path.as_ref().to_str().ok_or_else(|| {
                ZArchiveError::InvalidFilePath(path.as_ref().to_string_lossy().to_string())
            })?,
        )?))
    }

    pub fn read_file(&mut self, path: impl AsRef<std::path::Path>) -> Option<Vec<u8>> {
        let handle = self
            .0
            .pin_mut()
            .LookUp(path.as_ref().to_str()?, true, false)
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
                    return None;
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

        type ZArchiveReader;
        fn OpenFromFile(path: &str) -> Result<UniquePtr<ZArchiveReader>>;
        fn LookUp(
            self: Pin<&mut ZArchiveReader>,
            path: &str,
            allowFile: bool,
            allowDirectory: bool,
        ) -> Result<u32>;
        fn IsDirectory(&self, nodeHandle: u32) -> Result<bool>;
        fn IsFile(&self, nodeHandle: u32) -> Result<bool>;
        fn GetDirEntryCount(&self, nodeHandle: u32) -> Result<u32>;
        fn GetDirEntry<'a>(
            &'a self,
            nodeHandle: u32,
            index: u32,
            dirEntry: &'a mut DirEntry,
        ) -> Result<bool>;
        fn GetFileSize(self: Pin<&mut ZArchiveReader>, nodeHandle: u32) -> Result<u64>;
        unsafe fn ReadFromFile(
            self: Pin<&mut ZArchiveReader>,
            nodeHandle: u32,
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
            node_handle: u32,
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
