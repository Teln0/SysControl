#include <stdint.h>
#include <stddef.h>
#include <stivale2.h>

static uint8_t stack[65536];

__attribute__((section(".stivale2hdr"), used))
struct stivale2_header stivale_hdr = {
        .entry_point = 0,
        .stack = (uintptr_t)stack + sizeof(stack),
        .flags = 0
};

extern void kernel_main();

_Noreturn void _start(struct stivale2_struct *stivale2_struct) {
    kernel_main((uint64_t) stivale2_struct);

    // Hang
    for (;;) {
        asm ("hlt");
    }
}