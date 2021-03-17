#![feature(lang_items)]
#![feature(ptr_internals)]
#![feature(const_fn_fn_ptr_basics)]
#![feature(asm)]
#![feature(llvm_asm)]
#![feature(allocator_api)]
#![feature(alloc_error_handler)]
#![no_std]

use crate::memory::frame_allocator::{BitMapFrameAllocator, FrameAllocator, FRAME_SIZE, FrameInfo};
use crate::memory::paging::{EntryTable, EntryFlags};
use crate::utils::reg_write::write_cr3;
use crate::memory::heap::{LinkedListHeapAllocator, AllocOption};
use core::alloc::{Layout};
use alloc::boxed::Box;

extern crate rlibc;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate alloc;

#[macro_use]
pub mod display;
pub mod memory;
pub mod utils;

pub const KERNEL_OFFSET: usize = 0xffffffff80000000;
pub const MAX_HEAP: usize = 0x100000000; // 4GiB
pub const HEAP_OFFSET: usize = KERNEL_OFFSET - MAX_HEAP;

#[global_allocator]
static mut ALLOCATOR: AllocOption<LinkedListHeapAllocator<BitMapFrameAllocator>> = AllocOption(None);

#[no_mangle]
pub extern fn kernel_main(stivale_struct_ptr: usize) {
    println!("SysControl64 V0.2, booting up...");

    let stivale_struct = unsafe { stivale::load(stivale_struct_ptr) };


    println!("Bootloader info : {} {}",
             stivale_struct.bootloader_brand().expect("No bootloader brand provided."),
             stivale_struct.bootloader_version().expect("No bootloader version provided.")
    );

    let memory_map =
        stivale_struct.memory_map().expect("No memory map provided.");

    print!("Creating frame allocator... ");
    let mut frame_allocator = BitMapFrameAllocator::new(memory_map.iter());
    println!("Done !");

    print!("Marking VGA framebuffer as allocated... ");
    frame_allocator.mark_frame(0xb8000, true);
    println!("Done !");

    print!("Creating page tables... ");
    let p4_frame = frame_allocator.allocate_frame().expect("Out of memory (cannot create P4 page table).");
    // The frame allocator is guaranteed to return a valid frame
    let p4_table = unsafe {EntryTable::from_frame_unzeroed(p4_frame)};
    p4_table.zero();
    print!("[Created P4 table] ");
    p4_table.entries[511].write(p4_frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
    print!("[Recursively mapped P4 table to last entry] ");
    unsafe { p4_table.p4_kernel_remap(&stivale_struct, &mut frame_allocator); }
    print!("[Remapped the kernel] ");
    unsafe { write_cr3(p4_frame.address) };
    print!("[Switched to new page table] ");
    // p4 table is now accessed in a recursive way
    let p4_table = unsafe {
        EntryTable::from_frame_unzeroed(FrameInfo::from_address(0xffffffff_fffff000))
    };
    println!("Done !");
    print!("Creating kernel heap allocator... ");
    unsafe {
        let heap_allocator = LinkedListHeapAllocator::new(
            frame_allocator,
            p4_table,
            HEAP_OFFSET / FRAME_SIZE,
            MAX_HEAP
        );
        ALLOCATOR = AllocOption(Some(heap_allocator));
    }
    println!("Done !");

    for i in 0..1000 {
        let b = Box::new(i);
        assert_eq!(b.as_ref(), &i);
    }

    for i in 0..1000 {
        let b = Box::new(i);
        let b2 = Box::new(i * 2);
        let b3 = Box::new(i * 3);
        assert_eq!(b.as_ref(), &i);
        assert_eq!(b2.as_ref(), &(i * 2));
        assert_eq!(b3.as_ref(), &(i * 3));
    }

    for i in 0..1000 {
        let v = vec![i as usize; 1000];
        let b = Box::new(i);
        assert_eq!(b.as_ref(), &i);
    }

    println!("Ran 3000 allocation / deallocation tests !");
}

#[lang = "eh_personality"]
#[no_mangle]
pub extern fn eh_personality() {}

#[panic_handler]
pub extern fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    println!("{}", info);

    loop{}
}

#[alloc_error_handler]
pub fn alloc_error_handler(layout: Layout) -> ! {
    panic!("Failed to allocate the following layout : {:?}", layout);
}