use crate::memory::frame_allocator::{FrameAllocator, FRAME_SIZE};
use crate::memory::paging::{EntryTable, PageInfo, EntryFlags, TableAccess};
use core::alloc::{Layout, GlobalAlloc};
use crate::utils::ceil_div_usize;
use spin::Mutex;
use core::ops::DerefMut;
use crate::HEAP_OFFSET;

pub struct AllocOption<T> (pub Option<T>);

pub struct LinkedListHeapAllocatorInner {
    pub p4_table: &'static mut EntryTable,
    pub virtual_start_frame: usize,
    pub max_memory_amount: usize,
    pub max_currently_used: usize,
    pub holes: ListHeapNode
}

pub struct LinkedListHeapAllocator<T: FrameAllocator> {
    inner: Mutex<LinkedListHeapAllocatorInner>,
    frame_allocator: Mutex<T>
}

#[repr(packed)]
pub struct ListHeapNode {
    pub is_last: bool,

    pub next_node: usize,
    pub hole_size: usize
}

pub const LIST_HEAP_NODE_SIZE: usize = core::mem::size_of::<ListHeapNode>();

impl<T: FrameAllocator> LinkedListHeapAllocator<T> {
    pub unsafe fn new(
        frame_allocator: T,
        p4_table: &'static mut EntryTable,
        virtual_start_frame: usize,
        max_memory_amount: usize
    ) -> LinkedListHeapAllocator<T> {
        let max_currently_used = 0;

        let allocator = LinkedListHeapAllocator {
            inner: Mutex::new(LinkedListHeapAllocatorInner {
                p4_table,
                virtual_start_frame,
                max_memory_amount,
                max_currently_used,
                holes: ListHeapNode { is_last: true, next_node: 0, hole_size: 0 }
            }),
            frame_allocator: Mutex::new(frame_allocator)
        };

        allocator
    }
}

unsafe impl<T: FrameAllocator> GlobalAlloc for LinkedListHeapAllocator<T> {
    // TODO : take layout alignment in account
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut inner = self.inner.lock();
        let mut size = layout.size();
        // We don't want to leave micro holes when deallocating
        if size < LIST_HEAP_NODE_SIZE {
            size = LIST_HEAP_NODE_SIZE
        }
        // Searching for holes in the linked list
        let mut current = &mut inner.holes;
        let mut previous: Option<&mut ListHeapNode> = None;
        loop {
            // First hole will be always of size 0, so we can safely use it
            if current.hole_size >= size + LIST_HEAP_NODE_SIZE {
                // We found a suitable hole that won't leave any hole behind we couldn't fit a
                // linked list node into

                // to return
                let hole_address = (current as *const ListHeapNode) as usize;

                // placing the new node
                let new_node = &mut *((hole_address + size) as *mut ListHeapNode);
                *new_node = ListHeapNode {
                    is_last: current.is_last,
                    next_node: current.next_node,
                    hole_size: current.hole_size - size
                };

                // Set the next node in the previous node to the new one, since the hole was filled
                if let Some(previous) = previous {
                    previous.next_node = hole_address + size;
                }

                return hole_address as *mut u8;
            }

            // We searched up to the last hole didn't find anything suitable
            if current.is_last {
                break;
            }

            let next_node = current.next_node;
            previous = Some(current);
            current = &mut *(next_node as *mut ListHeapNode);
        }

        // We couldn't find any hole large enough
        let prev_max_currently_used = inner.max_currently_used;
        inner.max_currently_used += size;
        if inner.max_currently_used >= inner.max_memory_amount {
            // We are out of memory
            panic!("Reached maximum kernel heap size.");
        }
        let prev_frame = ceil_div_usize(prev_max_currently_used, FRAME_SIZE);
        let current_frame = ceil_div_usize(inner.max_currently_used, FRAME_SIZE);

        if prev_frame < current_frame {
            // Catching back with allocated and mapped frames
            for i in prev_frame..current_frame {
                // We need to allocate and map a new frame
                let mut frame_allocator = self.frame_allocator.lock();
                let frame_allocator = frame_allocator.deref_mut();
                let new_physical_frame = frame_allocator
                    .allocate_frame()
                    .expect("Out of memory (cannot get frame for heap allocator).");

                let page = PageInfo::from_number(inner.virtual_start_frame + i);

                inner.p4_table.p4_map(
                    new_physical_frame,
                    page,
                    EntryFlags::PRESENT | EntryFlags::WRITABLE,
                    false,
                    true,
                    TableAccess::Recursive,
                    frame_allocator
                );
            }
        }

        let ptr = (prev_max_currently_used + HEAP_OFFSET) as *mut u8;
        ptr
    }

    // TODO : free pages
    // TODO : take layout alignment in account
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut inner = self.inner.lock();
        let mut size = layout.size();
        // We didn't allow micro holes when allocating
        if size < LIST_HEAP_NODE_SIZE {
            size = LIST_HEAP_NODE_SIZE
        }

        // Find the right spot for the node in the linked list
        let mut current = &mut inner.holes;
        let first = current as *const ListHeapNode;
        loop {
            if current.is_last {
                let current_hole_address = (current as *const ListHeapNode) as usize;
                if current.hole_size + current_hole_address == ptr as usize && !core::ptr::eq(current, first) {
                    // We free a hole at the very end of the current one, so we can just extend
                    // the current one
                    current.hole_size += size;
                    return;
                }
                else {
                    // We append a hole to the hole list
                    current.is_last = false;
                    current.next_node = ptr as usize; // Hole will be placed on ptr
                    let new_hole = &mut *(ptr as *mut ListHeapNode);
                    *new_hole = ListHeapNode {
                        hole_size: size,
                        is_last: true,
                        next_node: 0
                    };
                    return;
                }
            }
            else {
                let current_hole_address = (current as *const ListHeapNode) as usize;
                if ptr as usize > current_hole_address {
                    // We found the right spot for our hole

                    let next_hole_address = current.next_node;
                    let next = &mut *(next_hole_address as *mut ListHeapNode);
                    let new_hole = &mut *(ptr as *mut ListHeapNode);
                    let new_hole_address = ptr as usize;

                    if current_hole_address + current.hole_size == ptr as usize && !core::ptr::eq(current, first) {
                        if new_hole_address + size == next_hole_address {
                            // Merge current with next
                            current.hole_size += size + next.hole_size;
                            current.next_node = next.next_node;
                        }
                        else {
                            // Merge new with current
                            current.hole_size += size;
                        }
                    }
                    else {
                        if new_hole_address + size == next_hole_address {
                            // Merge new with next
                            let is_last = next.is_last;
                            let after_next_address = next.next_node;
                            let size = size + next.hole_size;

                            current.next_node = new_hole_address;
                            *new_hole = ListHeapNode {
                                next_node: after_next_address,
                                is_last,
                                hole_size: size
                            };
                        }
                        else {
                            // Insert new hole without merging
                            current.next_node = new_hole_address;
                            *new_hole = ListHeapNode {
                                next_node: next_hole_address,
                                is_last: false,
                                hole_size: size
                            };
                        }
                    }

                    return;
                }
            }

            let next_node = current.next_node;
            current = &mut *(next_node as *mut ListHeapNode);
        }
    }
}

unsafe impl<T: FrameAllocator> GlobalAlloc for AllocOption<LinkedListHeapAllocator<T>> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if let Some(alloc) = &self.0 {
            alloc.alloc(layout)
        }
        else {
            panic!("Tried using heap allocator before initializing it.");
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(alloc) = &self.0 {
            alloc.dealloc(ptr, layout)
        }
        else {
            panic!("Tried using heap allocator before initializing it.");
        }
    }
}