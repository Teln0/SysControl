pub fn ceil_div_usize(a: usize, b: usize) -> usize {
    (a + b - 1) / b
}

pub fn mem_regions_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start.max(b_start) <= a_end.min(b_end)
}

pub mod reg_read {
    pub unsafe fn read_cr3() -> usize {
        let result: u64;
        asm!("mov {}, cr3", out(reg) result);
        result as usize
    }
}

pub mod reg_write {
    pub unsafe fn write_cr3(to_write: usize) {
        let to_write = to_write as u64;
        asm!("mov cr3, {}", in(reg) to_write);
    }
}

/*
pub mod cpu_features {
    use x86_64::registers::control::{EferFlags, Cr0, Cr0Flags};
    use x86_64::registers::model_specific::Efer;

    pub unsafe fn enable_nxe_x86_64() {
        let mut efer = Efer::read();
        efer.set(EferFlags::NO_EXECUTE_ENABLE, true);
        Efer::write(efer);
    }

    pub unsafe fn enable_write_protect_x86_64() {
        let mut cr0 = Cr0::read();
        cr0.set(Cr0Flags::WRITE_PROTECT, true);
        Cr0::write(cr0);
    }
}
*/