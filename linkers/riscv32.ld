ENTRY(_start)

SECTIONS {
. = 0x80200000;

.text : {
    __text_start = .;
    *(.text*)
    . = ALIGN(4096);
    __text_end = .;
}

.rodata : {
    __rodata_start = .;
    *(.rodata*)
    . = ALIGN(4096);
    __rodata_end = .;
}

.data : {
    __data_start = .;
    *(.data*)
    . = ALIGN(4096);
    __data_end = .;
}

.bss : {
    __bss_start = .;
    *(.bss*)
    . = ALIGN(4096);
    __bss_end = .;
}

. = ALIGN(4096);
__end = .;


}