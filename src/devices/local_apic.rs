use crate::paging;

pub struct LocalApicAccess {
    mapping: paging::Region,
}

impl LocalApicAccess {
    pub unsafe fn new() -> Self {
        use x86::msr::*;

        let physical_address = rdmsr(IA32_APIC_BASE) as usize & 0xffff_0000;
        let mapping = paging::map_physical_memory(
            physical_address,
            paging::PAGE_SIZE,
            paging::PhysicalMappingFlags::UNCACHED,
        )
        .expect("Failed to map local apic");

        Self { mapping }
    }

    pub unsafe fn read(&self, offset: u16) -> u32 {
        core::intrinsics::volatile_load(self.mapping.as_ptr_offset(offset.into()))
    }

    unsafe fn write(&mut self, offset: u16, value: u32) {
        core::intrinsics::volatile_store(self.mapping.as_mut_ptr_offset(offset.into()), value)
    }

    pub fn id(&self) -> u32 {
        unsafe { self.read(0x20) }
    }

    pub fn set_icr(&mut self, value: u64) {
        unsafe {
            while self.read(0x300) & 1 << 12 == 1 << 12 {}
            self.write(0x310, (value >> 32) as u32);
            self.write(0x300, value as u32);
            while self.read(0x300) & 1 << 12 == 1 << 12 {}
        }
    }
}

static mut LOCAL_APIC_ACCESS: Option<LocalApicAccess> = None;

pub fn local_apic_access<'a>() -> &'a mut LocalApicAccess {
    unsafe { LOCAL_APIC_ACCESS.as_mut().unwrap() }
}

pub fn local_apic_access_safe<'a>() -> Option<&'a mut LocalApicAccess> {
    unsafe { LOCAL_APIC_ACCESS.as_mut() }
}

fn disable_pic() {
    use crate::io_port::{Io, IoPort};

    // We have to disable the PIC. We never want to hear from it. But, to be safe, we configure it
    // first, then disable it.
    let mut master_cmd: IoPort<u8> = IoPort::new(0x20);
    let mut master_data: IoPort<u8> = IoPort::new(0x21);
    let mut slave_cmd: IoPort<u8> = IoPort::new(0xa0);
    let mut slave_data: IoPort<u8> = IoPort::new(0xa1);

    // Start initialization
    master_cmd.write(0x11);
    slave_cmd.write(0x11);

    // Set offsets
    master_data.write(0x20);
    slave_data.write(0x28);

    // Set up cascade
    master_data.write(4);
    slave_data.write(2);

    // Set up interrupt mode
    master_data.write(1);
    slave_data.write(1);

    // Mask all interrupts
    master_data.write(0xff);
    slave_data.write(0xff);

    // Ack remaining interrupts
    master_cmd.write(0x20);
    slave_cmd.write(0x20);
}

pub unsafe fn init_bsp() {
    // Before doing anything else, disable the PIC so it doesn't get in the way
    disable_pic();

    // Set up the local apic access object. This does not need to be per core because
    // the mechanics of accessing the local apic do not change between cores.
    LOCAL_APIC_ACCESS = Some(LocalApicAccess::new());

    // Set the spurious interrupt register to 0xff and enable the local APIC
    local_apic_access().write(0xf0, 0x1ff);
}

pub unsafe fn init_ap() {
    // Set the spurious interrupt register to 0xff and enable the local APIC
    local_apic_access().write(0xf0, 0x1ff);
}
