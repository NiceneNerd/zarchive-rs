#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("zarchive/include/zarchive/zarchivewriter.h");
    }
}
