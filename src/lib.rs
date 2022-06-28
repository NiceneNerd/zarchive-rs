static ZARCHIVE_INVALID_NODE: u32 = 0xFFFFFFFF;

#[cxx::bridge]
mod ffi {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_zarchive() {
        let mut result = ffi::OpenFromFile("test/crafting.zar").unwrap();
        println!("Opened archive");
        println!(
            "Did we find it? {:?}",
            result
                .pin_mut()
                .LookUp("content/Pack/Bootup.pack", true, false)
                .unwrap()
                != ZARCHIVE_INVALID_NODE
        );
    }
}
