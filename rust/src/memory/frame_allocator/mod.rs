use crate::utils::{ceil_div_usize};
use stivale::memory::MemoryMapIter;
use stivale::memory::MemoryMapEntryType::Usable;
use crate::memory::paging::{EntryTable, PageInfo, EntryFlags, TableAccess};

pub const FRAME_SIZE: usize = 4096;

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct FrameInfo {
    pub number: usize,
    pub address: usize
}

impl FrameInfo {
    pub fn from_address(address: usize) -> FrameInfo {
        FrameInfo {
            number: address / FRAME_SIZE,
            address
        }
    }

    pub fn from_number(number: usize) -> FrameInfo {
        FrameInfo {
            number,
            address: number * FRAME_SIZE
        }
    }
}

pub trait FrameAllocator {
    fn allocate_frame(&mut self) -> Option<FrameInfo>;
    fn deallocate_frame(&mut self, frame_info: FrameInfo);
    unsafe fn identity_map(
        &mut self,
        p4_table: &mut EntryTable,
        invalidate_addresses: bool,
        current_table_access: TableAccess
    );
}

#[derive(Debug)]
pub struct BitMapFrameAllocator {
    pub bitmap_frame: usize,
    pub bitmap_size_in_bytes: usize,
    pub bitmap_size_in_frames: usize,
    pub frames_amount: usize,
    pub memory_end: usize,
    pub slice: &'static mut[u8]
}

impl BitMapFrameAllocator {
    pub fn mark_frame(&mut self, frame: usize, allocated: bool) {
        if frame > self.memory_end {
            panic!("Cannot mark region above memory end as allocated.");
        }

        let byte = frame / 8;
        let bit_index = frame % 8;

        if allocated {
            self.slice[byte] |= 1 << bit_index;
        }
        else {
            self.slice[byte] &= !(1 << bit_index);
        }
    }

    pub fn mark_region(&mut self, start: usize, end: usize, allocated: bool) {
        let frame_start = start / FRAME_SIZE;
        let frame_end = ceil_div_usize(end, FRAME_SIZE);

        for i in frame_start..frame_end {
            self.mark_frame(i, allocated);
        }
    }

    pub fn clear_bitmap(&mut self) {
        for i in 0..self.bitmap_size_in_bytes {
            self.slice[i] = 0;
        }
    }

    pub fn new(areas: MemoryMapIter) -> BitMapFrameAllocator {
        let memory_end = areas.clone().max_by(|entry_1, entry_2| entry_1.end_address().cmp(&entry_2.end_address())).unwrap().end_address() as usize;

        let mut areas = areas.clone();
        let mut areas_2 = areas.clone();
        let frames_amount = memory_end / FRAME_SIZE; // Discard any incomplete frame at the end of memory
        let bitmap_length_in_bytes = ceil_div_usize(frames_amount, 8);

        // Find continuous frames of at least bitmap_length_in_bytes
        let continuous_frames_amount = ceil_div_usize(bitmap_length_in_bytes, FRAME_SIZE);

        let mut found = false;
        let mut tested_mem_area = areas.next().unwrap();
        let mut tested_frame = ceil_div_usize(tested_mem_area.start_address() as usize, FRAME_SIZE);

        while !found {
            // Checking if we fit into the mem area
            let usable = match tested_mem_area.entry_type() {
                Usable => true,
                _ => false
            };

            if !usable ||
                (tested_frame + continuous_frames_amount + 1) * FRAME_SIZE > (tested_mem_area.start_address() + tested_mem_area.size()) as usize {
                tested_mem_area = if let Some(area) = areas.next() {
                    area
                }
                else {
                    panic!("Could not find sufficiently big memory region to allocate bitmap.");
                };
                tested_frame = ceil_div_usize(tested_mem_area.start_address() as usize, FRAME_SIZE);
            }
            else {
                found = true;
            }
        }

        let bitmap_ptr = (tested_frame * FRAME_SIZE) as *mut u8;
        let slice: &mut[u8] = unsafe {core::slice::from_raw_parts_mut::<'static>(bitmap_ptr, bitmap_length_in_bytes)};

        let mut allocator = BitMapFrameAllocator {
            frames_amount,
            bitmap_frame: tested_frame,
            bitmap_size_in_bytes: bitmap_length_in_bytes,
            bitmap_size_in_frames: continuous_frames_amount,
            memory_end,
            slice
        };

        // Clear leftover stuff
        allocator.clear_bitmap();

        // Mark region used by bitmap
        allocator.mark_region(tested_frame * FRAME_SIZE, tested_frame * FRAME_SIZE + bitmap_length_in_bytes, true);

        // Mark unavailable memory regions as allocated
        let areas = areas_2.clone();
        for area in areas {
            let usable = match area.entry_type() {
                Usable => true,
                _ => false
            };
            if !usable {
                allocator.mark_region(area.start_address() as usize, area.end_address() as usize, true);
            }
        }

        // Mark non-present memory regions as allocated
        let mut previous = areas_2.next().unwrap();
        while let Some(current) = areas_2.next() {
            allocator.mark_region(previous.end_address() as usize, current.start_address() as usize, true);
            previous = current;
        }

        allocator
    }
}

impl FrameAllocator for BitMapFrameAllocator {
    fn allocate_frame(&mut self) -> Option<FrameInfo> {

        // Find a non-full byte
        let mut byte = 0;
        while self.slice[byte] == u8::MAX {
            byte += 1;
            if byte >= self.slice.len() {
                return None; // We are out of memory
            }
        }

        // We find the first free bit in the byte
        // Processors this os runs on are little endian so the trailing ones will be the first ones of the byte
        let trailing_ones = self.slice[byte].trailing_ones() as usize;
        let index = byte * 8 + trailing_ones;
        if index >= self.frames_amount {
            return None; // We are out of memory
        }

        self.mark_frame(index, true);

        Some(FrameInfo {
            number: index,
            address: index * FRAME_SIZE
        })
    }

    fn deallocate_frame(&mut self, frame_info: FrameInfo) {
        self.mark_frame(frame_info.number, false);
    }

    unsafe fn identity_map(
        &mut self,
        p4_table: &mut EntryTable,
        invalidate_addresses: bool,
        current_table_access: TableAccess
    ) {
        let start_frame = self.bitmap_frame;
        let end_frame = self.bitmap_frame + self.bitmap_size_in_frames;
        let flags = EntryFlags::PRESENT | EntryFlags::WRITABLE;
        for i in start_frame..end_frame {
            p4_table.p4_map(
                FrameInfo::from_number(i),
                PageInfo::from_number(i),
                flags,
                false,
                invalidate_addresses,
                current_table_access,
                self
            );
        }
    }
}