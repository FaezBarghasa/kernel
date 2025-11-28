//! # Linker Symbols (AArch64)

extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __bss_start: u8;
    static __bss_end: u8;
    static __usercopy_start: u8;
    static __usercopy_end: u8;
}

fn symbol_addr(sym: &u8) -> usize {
    unsafe { sym as *const u8 as usize }
}

pub fn text_start() -> usize {
    symbol_addr(unsafe { &__text_start })
}
pub fn text_end() -> usize {
    symbol_addr(unsafe { &__text_end })
}
pub fn rodata_start() -> usize {
    symbol_addr(unsafe { &__rodata_start })
}
pub fn rodata_end() -> usize {
    symbol_addr(unsafe { &__rodata_end })
}
pub fn data_start() -> usize {
    symbol_addr(unsafe { &__data_start })
}
pub fn data_end() -> usize {
    symbol_addr(unsafe { &__data_end })
}
pub fn bss_start() -> usize {
    symbol_addr(unsafe { &__bss_start })
}
pub fn bss_end() -> usize {
    symbol_addr(unsafe { &__bss_end })
}
pub fn usercopy_start() -> usize {
    symbol_addr(unsafe { &__usercopy_start })
}
pub fn usercopy_end() -> usize {
    symbol_addr(unsafe { &__usercopy_end })
}

