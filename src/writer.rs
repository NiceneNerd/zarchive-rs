use crate::{Result, ZArchiveError};
use std::path::Path;

pub fn pack(input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<()> {
    let input = input.as_ref();
    let output = output.as_ref();
    if !input.exists() || !input.is_dir() {
        return Err(ZArchiveError::IOError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Input file not found or not a directory",
        )));
    }
    if output.exists() {
        std::fs::remove_file(&output)?;
    } else if !output.parent().unwrap().exists() {
        std::fs::create_dir_all(output.parent().unwrap())?;
    }
    ffi::Pack(
        input
            .to_str()
            .ok_or_else(|| ZArchiveError::InvalidFilePath(input.to_string_lossy().to_string()))?,
        output
            .to_str()
            .ok_or_else(|| ZArchiveError::InvalidFilePath(output.to_string_lossy().to_string()))?,
    )?;
    Ok(())
}
#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("zarchive/include/zarchive/zarchivewriter.h");

        fn Pack(inputPath: &str, outputPath: &str) -> Result<()>;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pack() {
        super::pack("test/crafting", "test/crafting2.zar").unwrap();
        let reader = crate::ZArchiveReader::new("test/crafting.zar").unwrap();
        let reader2 = crate::ZArchiveReader::new("test/crafting2.zar").unwrap();
        assert_eq!(reader.get_files().unwrap(), reader2.get_files().unwrap());
    }
}
