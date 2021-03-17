use crate::memory::frame_allocator::{FrameInfo, FRAME_SIZE, FrameAllocator};
use crate::utils::reg_write::write_cr3;
use crate::utils::reg_read::read_cr3;
use stivale::StivaleStructure;
use stivale::memory::MemoryMapEntryType;
use crate::utils::ceil_div_usize;
bitflags! {
    pub struct EntryFlags: u64 {
        const PRESENT =         1 << 0;
        const WRITABLE =        1 << 1;
        const USER_ACCESSIBLE = 1 << 2;
        const WRITE_THROUGH =   1 << 3;
        const NO_CACHE =        1 << 4;
        const ACCESSED =        1 << 5;
        const DIRTY =           1 << 6;
        const HUGE_PAGE =       1 << 7;
        const GLOBAL =          1 << 8;
        const NO_EXECUTE =      1 << 63;
    }
}

#[derive(Clone, Copy)]
pub enum TableAccess {
    Recursive,
    Identity
}

pub struct PageInfo {
    pub number: usize,
    pub address: usize
}

impl PageInfo {
    fn p4_index(&self) -> usize {
        (self.number >> 27) & 0o777
    }
    fn p3_index(&self) -> usize {
        (self.number >> 18) & 0o777
    }
    fn p2_index(&self) -> usize {
        (self.number >> 9) & 0o777
    }
    fn p1_index(&self) -> usize {
        (self.number >> 0) & 0o777
    }

    pub fn from_address(virtual_address: usize) -> PageInfo {
        assert!(virtual_address < 0x0000_8000_0000_0000 ||
                    virtual_address >= 0xffff_8000_0000_0000,
                "invalid address: 0x{:x}", virtual_address);
        let number = virtual_address / FRAME_SIZE;
        let address = number * FRAME_SIZE;
        PageInfo { number, address }
    }

    pub fn from_number(number: usize) -> PageInfo {
        PageInfo { number, address: number * FRAME_SIZE }
    }
}

#[derive(Copy, Clone)]
pub struct Entry (pub u64);

impl Entry {
    pub fn set_unused(&mut self) {
        self.0 = 0;
    }

    pub fn is_unused(&self) -> bool {
        self.0 == 0
    }

    pub fn get_flags(&self) -> EntryFlags {
        EntryFlags::from_bits_truncate(self.0)
    }

    pub fn read_address(&self) -> usize {
        (self.0 & 0x000fffff_fffff000) as usize
    }

    pub fn write(&mut self, frame: FrameInfo, flags: EntryFlags) {
        assert_eq!(frame.address & !0x000fffff_fffff000, 0);
        self.0 = (frame.address as u64) | flags.bits();
    }

    pub fn pointed_frame(&self) -> Option<FrameInfo> {
        if self.get_flags().contains(EntryFlags::PRESENT) {
            Some(FrameInfo::from_address(self.read_address()))
        }
        else {
            None
        }
    }
}

#[repr(align(4096))]
pub struct EntryTable {
    pub entries: [Entry; 512]
}

impl EntryTable {
    pub fn zero(&mut self) {
        for i in 0..512 {
            self.entries[i].set_unused();
        }
    }

    pub unsafe fn from_frame_unzeroed(frame: FrameInfo) -> &'static mut EntryTable {
        &mut *(frame.address as *mut EntryTable)
    }

    fn next_entry_address_recursive(&self, index: usize) -> Option<usize> {
        let entry_flags = self.entries[index].get_flags();
        if entry_flags.contains(EntryFlags::PRESENT) && !entry_flags.contains(EntryFlags::HUGE_PAGE) {
            let table_address = self as *const _ as usize;
            Some((table_address << 9) | (index << 12))
        } else {
            None
        }
    }

    pub unsafe fn p4_map<T: FrameAllocator>(
        &mut self,
        frame: FrameInfo,
        page: PageInfo,
        flags: EntryFlags,
        allow_overwrite: bool,
        invalidate_addres: bool,
        current_table_access: TableAccess,
        allocator: &mut T
    ) {
        match current_table_access {
            // In this case tables are identity mapped so their physical address, the one found with
            // .pointed_frame() is their virtual address as well.
            TableAccess::Identity => {
                // P3 table
                let table = EntryTable::from_frame_unzeroed(
                    self.create_or_get_table_entry(
                        page.p4_index(),
                        TableAccess::Identity,
                        allocator
                    ).pointed_frame().unwrap()
                );
                // P2 table
                let table = EntryTable::from_frame_unzeroed(
                    table.create_or_get_table_entry(
                        page.p3_index(),
                        TableAccess::Identity,
                        allocator
                    ).pointed_frame().unwrap()
                );
                // P1 table
                let table = EntryTable::from_frame_unzeroed(
                    table.create_or_get_table_entry(
                        page.p2_index(),
                        TableAccess::Identity,
                        allocator
                    ).pointed_frame().unwrap()
                );
                // Setting the entry
                let entry: &mut Entry = &mut table.entries[page.p1_index()];
                if !entry.is_unused() {
                    if !allow_overwrite {
                        panic!("Tried to perform unauthorized entry overwrite.");
                    }
                    // We changed something
                    if invalidate_addres {
                        invalidate(page.address);
                    }
                }
                entry.write(frame, flags | EntryFlags::PRESENT);
            }

            // In this case tables are mapped recursively, the last entry of the P4 table leads to
            // itself
            TableAccess::Recursive => {
                // P3 table
                let table = {
                    self.create_or_get_table_entry(
                        page.p4_index(),
                        TableAccess::Recursive,
                        allocator
                    );

                    EntryTable::from_frame_unzeroed(FrameInfo::from_address(self.next_entry_address_recursive(
                        page.p4_index()
                    ).expect("An error occurred while creating the page table")))
                };

                // P2 table
                let table = {
                    table.create_or_get_table_entry(
                        page.p3_index(),
                        TableAccess::Recursive,
                        allocator
                    );

                    EntryTable::from_frame_unzeroed(FrameInfo::from_address(table.next_entry_address_recursive(
                        page.p3_index()
                    ).expect("An error occurred while creating the page table")))
                };

                // P1 table
                let table = {
                    table.create_or_get_table_entry(
                        page.p2_index(),
                        TableAccess::Recursive,
                        allocator
                    );

                    EntryTable::from_frame_unzeroed(FrameInfo::from_address(table.next_entry_address_recursive(
                        page.p2_index()
                    ).expect("An error occurred while creating the page table")))
                };
                // Setting the entry
                let entry: &mut Entry = &mut table.entries[page.p1_index()];
                if !entry.is_unused() {
                    if !allow_overwrite {
                        panic!("Tried to perform unauthorized entry overwrite.");
                    }
                    // We changed something
                    if invalidate_addres {
                        invalidate(page.address);
                    }
                }
                entry.write(frame, flags | EntryFlags::PRESENT);
            }
        }
    }

    // TODO : Optimize
    pub fn create_or_get_table_entry<T: FrameAllocator>(
        &mut self,
        index: usize,
        current_table_access: TableAccess,
        allocator: &mut T
    ) -> &mut Entry {
        if self.entries[index].is_unused() {
            // The frame allocator is guaranteed to return a valid frame
            let frame = allocator.allocate_frame().expect("Out of memory (cannot create page table).");
            self.entries[index].write(frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);

            let new_table = match current_table_access {
                TableAccess::Recursive => {
                    unsafe { EntryTable::from_frame_unzeroed(
                        FrameInfo::from_address(
                            self.next_entry_address_recursive(index)
                                .expect("Failed to create new table.")
                        )
                    ) }
                }
                TableAccess::Identity => {
                    unsafe { EntryTable::from_frame_unzeroed(frame) }
                }
            };
            new_table.zero();
            &mut self.entries[index]
        }
        else {
            &mut self.entries[index]
        }
    }

    pub unsafe fn p4_kernel_remap<T: FrameAllocator>(
        &mut self,
        stivale_structure: &StivaleStructure,
        allocator: &mut T
    ) {
        let memory_map = stivale_structure.memory_map().expect(
            "No memory map provided."
        );

        // Making sure the frame allocator identity maps all of its data
        allocator.identity_map(
            self,
            false,
            TableAccess::Identity
        );

        // Map the VGA framebuffer
        self.p4_map(
            FrameInfo::from_address(0xb8000),
            PageInfo::from_address(0xb8000),
            EntryFlags::PRESENT | EntryFlags::WRITABLE,
            false,
            false,
            TableAccess::Identity,
            allocator
        );

        for i in memory_map.iter() {
            // Some memory areas need to be mapped, some don't
            let (do_map, offset) = match i.entry_type() {
                MemoryMapEntryType::Usable => (false, 0usize),
                MemoryMapEntryType::Reserved => (true, 0usize),
                MemoryMapEntryType::AcpiReclaimable => (true, 0usize),
                MemoryMapEntryType::AcpiNvs => (true, 0usize),
                MemoryMapEntryType::BadMemory => (false, 0usize),
                MemoryMapEntryType::BootloaderReclaimable => (true, 0usize),
                MemoryMapEntryType::Kernel => (true, 0xffffffff80000000usize)
            };

            if do_map {
                let frame_start = i.start_address() as usize / FRAME_SIZE;
                let frame_end = ceil_div_usize(i.end_address() as usize, FRAME_SIZE);
                let frame_offset = offset / FRAME_SIZE;

                let region_size = frame_end - frame_start;

                let flags = EntryFlags::PRESENT | EntryFlags::WRITABLE;

                for frame in 0..region_size {
                    self.p4_map(
                        FrameInfo::from_number(frame_start + frame),
                        PageInfo::from_number(frame_start + frame + frame_offset),
                        flags,
                        false,
                        false,
                        TableAccess::Identity,
                        allocator
                    );
                }
            }
        }
    }
}

pub fn invalidate(virtual_address: usize) {
    unsafe { llvm_asm!("invlpg ($0)" :: "r" (virtual_address) : "memory") };
}

pub fn invalidate_all() {
    unsafe {
        write_cr3(read_cr3());
    }
}