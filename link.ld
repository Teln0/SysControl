ENTRY(_start)

SECTIONS
{
    /* We want to be placed in the higher half, 2MiB above 0 in physical memory. */
    . = 0xffffffff80200000;

    /* We place the .stivale2hdr section containing the header in its own section, */
    /* and we use the KEEP directive on it to make sure it doesn't get discarded. */
    .stivale2hdr : {
        KEEP(*(.stivale2hdr))
    }



    .text : {
        *(.text*)
    }

    .rodata : {
        *(.rodata*)
    }

    .data : {
        *(.data*)
    }

    .bss : {
        *(COMMON)
        *(.bss*)
    }
}