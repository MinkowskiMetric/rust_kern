use crate::acpi::ACPI;
use crate::paging;
use acpi::interrupt::InterruptModel;
use alloc::vec::Vec;
use core::fmt;
use spin::Mutex;

struct IoApicRegisters {
    mapping: paging::Region,
}

impl IoApicRegisters {
    pub unsafe fn new(address: usize) -> Option<Self> {
        paging::map_physical_memory(address, 8, paging::PhysicalMappingFlags::UNCACHED)
            .ok()
            .map(|mapping| Self { mapping })
    }

    fn ioregsel(&self) -> *mut u32 {
        self.mapping.as_ptr_offset::<u32>(0) as *mut u32
    }

    fn iowin(&self) -> *mut u32 {
        self.mapping.as_ptr_offset::<u32>(0x10) as *mut u32
    }

    /*fn read_ioregsel(&self) -> u32 {
        unsafe { core::ptr::read_volatile(self.ioregsel()) }
    }*/

    fn write_ioregsel(&self, value: u32) {
        unsafe {
            core::ptr::write_volatile(self.ioregsel(), value);
        }
    }

    fn read_iowin(&self) -> u32 {
        unsafe { core::ptr::read_volatile(self.iowin()) }
    }

    fn write_iowin(&self, value: u32) {
        unsafe {
            core::ptr::write_volatile(self.iowin(), value);
        }
    }

    fn read_reg(&self, reg: u8) -> u32 {
        self.write_ioregsel(reg.into());
        self.read_iowin()
    }

    fn write_reg(&mut self, reg: u8, value: u32) {
        self.write_ioregsel(reg.into());
        self.write_iowin(value);
    }

    pub fn read_ioapicid(&self) -> u32 {
        self.read_reg(0x00)
    }
    /*pub fn write_ioapicid(&mut self, value: u32) {
        self.write_reg(0x00, value);
    }*/
    pub fn read_ioapicver(&mut self) -> u32 {
        self.read_reg(0x01)
    }
    /*pub fn read_ioapicarb(&mut self) -> u32 {
        self.read_reg(0x02)
    }*/
    pub fn read_ioredtbl(&mut self, idx: u8) -> u64 {
        assert!(idx < 24);
        let lo = self.read_reg(0x10 + idx * 2);
        let hi = self.read_reg(0x10 + idx * 2 + 1);

        u64::from(lo) | (u64::from(hi) << 32)
    }
    pub fn write_ioredtbl(&mut self, idx: u8, value: u64) {
        assert!(idx < 24);

        let lo = value as u32;
        let hi = (value >> 32) as u32;

        self.write_reg(0x10 + idx * 2, lo);
        self.write_reg(0x10 + idx * 2 + 1, hi);
    }

    pub fn max_redirection_table_entries(&mut self) -> u8 {
        let ver = self.read_ioapicver();
        ((ver & 0x00FF_0000) >> 16) as u8
    }
    pub fn id(&mut self) -> u8 {
        let id_reg = self.read_ioapicid();
        ((id_reg & 0x0F00_0000) >> 24) as u8
    }
}

pub struct IoApic {
    registers: Mutex<IoApicRegisters>,
    count: u8,
    global_system_interrupt_base: u32,
}

impl IoApic {
    pub unsafe fn new(address: usize, id: u8, global_system_interrupt_base: u32) -> Option<Self> {
        IoApicRegisters::new(address).map(|mut registers| {
            assert_eq!(registers.id(), id, "IOAPIC ID doesn't match ACPI");

            let count = registers.max_redirection_table_entries();

            Self {
                registers: Mutex::new(registers),
                count,
                global_system_interrupt_base,
            }
        })
    }

    /// Map an interrupt vector to a physical local APIC ID of a processor (thus physical mode).
    pub fn map(&self, idx: u8, info: MapInfo) {
        self.registers.lock().write_ioredtbl(idx, info.as_raw())
    }
    pub fn set_mask(&self, global_system_interrupt: u32, mask: bool) {
        let idx = (global_system_interrupt - self.global_system_interrupt_base) as u8;
        let mut guard = self.registers.lock();

        let mut reg = guard.read_ioredtbl(idx);
        reg &= !(1 << 16);
        reg |= u64::from(mask) << 16;
        guard.write_ioredtbl(idx, reg);
    }
}

impl fmt::Debug for IoApic {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        struct RedirTable<'a>(&'a Mutex<IoApicRegisters>);

        impl<'a> fmt::Debug for RedirTable<'a> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                let mut guard = self.0.lock();

                let count = guard.max_redirection_table_entries();
                f.debug_list()
                    .entries((0..count).map(|i| guard.read_ioredtbl(i)))
                    .finish()
            }
        }

        f.debug_struct("IoApic")
            .field("redir_table", &RedirTable(&self.registers))
            .field(
                "global_system_interrupt_base",
                &self.global_system_interrupt_base,
            )
            .field("count", &self.count)
            .finish()
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Polarity {
    SameAsBus,
    ActiveHigh,
    ActiveLow,
}

impl<'a> From<&'a acpi::interrupt::Polarity> for Polarity {
    fn from(other: &'a acpi::interrupt::Polarity) -> Self {
        match other {
            acpi::interrupt::Polarity::SameAsBus => Self::SameAsBus,
            acpi::interrupt::Polarity::ActiveHigh => Self::ActiveHigh,
            acpi::interrupt::Polarity::ActiveLow => Self::ActiveLow,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum TriggerMode {
    SameAsBus,
    Edge,
    Level,
}

impl<'a> From<&'a acpi::interrupt::TriggerMode> for TriggerMode {
    fn from(other: &'a acpi::interrupt::TriggerMode) -> Self {
        match other {
            acpi::interrupt::TriggerMode::SameAsBus => Self::SameAsBus,
            acpi::interrupt::TriggerMode::Edge => Self::Edge,
            acpi::interrupt::TriggerMode::Level => Self::Level,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Override {
    pub isa_source: u8,
    pub global_system_interrupt: u32,
    pub polarity: Polarity,
    pub trigger_mode: TriggerMode,
}

impl<'a> From<&'a acpi::interrupt::InterruptSourceOverride> for Override {
    fn from(iso: &'a acpi::interrupt::InterruptSourceOverride) -> Self {
        Self {
            isa_source: iso.isa_source,
            global_system_interrupt: iso.global_system_interrupt,
            polarity: (&iso.polarity).into(),
            trigger_mode: (&iso.trigger_mode).into(),
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum ApicTriggerMode {
    Edge = 0,
    Level = 1,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum ApicPolarity {
    ActiveHigh = 0,
    ActiveLow = 1,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum DestinationMode {
    Physical = 0,
    Logical = 1,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum DeliveryMode {
    Fixed = 0b000,
    LowestPriority = 0b001,
    Smi = 0b010,
    Nmi = 0b100,
    Init = 0b101,
    ExtInt = 0b111,
}

#[derive(Clone, Copy, Debug)]
pub struct MapInfo {
    pub dest: u8,
    pub mask: bool,
    pub trigger_mode: ApicTriggerMode,
    pub polarity: ApicPolarity,
    pub dest_mode: DestinationMode,
    pub delivery_mode: DeliveryMode,
    pub vector: u8,
}

impl MapInfo {
    pub fn as_raw(&self) -> u64 {
        assert!(self.vector >= 0x20);
        assert!(self.vector <= 0xFE);

        // TODO: Check for reserved fields.

        (u64::from(self.dest) << 56)
            | (u64::from(self.mask) << 16)
            | ((self.trigger_mode as u64) << 15)
            | ((self.polarity as u64) << 13)
            | ((self.dest_mode as u64) << 11)
            | ((self.delivery_mode as u64) << 8)
            | u64::from(self.vector)
    }
}

static mut IOAPICS: Option<Vec<IoApic>> = None;
static mut SRC_OVERRIDES: Option<Vec<Override>> = None;

pub unsafe fn init() {
    let bsp_apic_id = x86::cpuid::CpuId::new()
        .get_feature_info()
        .unwrap()
        .initial_local_apic_id();

    let mut acpi_lock = ACPI.lock();
    let acpi = acpi_lock.as_mut().unwrap();

    let interrupt_model = match &acpi.acpi_context.interrupt_model {
        Some(InterruptModel::Apic(apic)) => apic,
        _ => panic!("Unsupported interrupt model"),
    };

    for io_apic in interrupt_model.io_apics.iter() {
        if let Some(io_apic) = IoApic::new(
            io_apic.address as usize,
            io_apic.id,
            io_apic.global_system_interrupt_base,
        ) {
            IOAPICS.get_or_insert_with(Vec::new).push(io_apic);
        } else {
            panic!("Failed to initialize io_apic id {:#x}", io_apic.id);
        }
    }

    SRC_OVERRIDES = Some(
        interrupt_model
            .interrupt_source_overrides
            .iter()
            .map(|iso| iso.into())
            .collect(),
    );

    // map the legacy PC-compatible IRQs (0-15) to 32-47, just like we did with 8259 PIC (if it
    // wouldn't have been disabled due to this I/O APIC)
    for legacy_irq in 0..=15 {
        let (global_system_interrupt, trigger_mode, polarity) = match get_src_override(legacy_irq) {
            Some(over) => (
                over.global_system_interrupt,
                over.trigger_mode,
                over.polarity,
            ),
            None => {
                if src_overrides().iter().any(|over| {
                    over.global_system_interrupt == u32::from(legacy_irq)
                        && over.isa_source != legacy_irq
                }) && !src_overrides()
                    .iter()
                    .any(|over| over.isa_source == legacy_irq)
                {
                    // there's an IRQ conflict, making this legacy IRQ inaccessible.
                    continue;
                }
                (
                    legacy_irq.into(),
                    TriggerMode::SameAsBus,
                    Polarity::SameAsBus,
                )
            }
        };

        let apic = match find_ioapic(global_system_interrupt) {
            Some(ioapic) => ioapic,
            None => {
                crate::println!("Unable to find a suitable APIC for legacy IRQ {} (GSI {}). It will not be mapped.", legacy_irq, global_system_interrupt);
                continue;
            }
        };

        let redir_tbl_index = (global_system_interrupt - apic.global_system_interrupt_base) as u8;

        let map_info = MapInfo {
            // only send to the BSP
            dest: bsp_apic_id,
            dest_mode: DestinationMode::Physical,
            delivery_mode: DeliveryMode::Fixed,
            mask: false,
            polarity: match polarity {
                Polarity::ActiveHigh => ApicPolarity::ActiveHigh,
                Polarity::ActiveLow => ApicPolarity::ActiveLow,
                Polarity::SameAsBus => ApicPolarity::ActiveHigh,
            },
            trigger_mode: match trigger_mode {
                TriggerMode::Edge => ApicTriggerMode::Edge,
                TriggerMode::Level => ApicTriggerMode::Level,
                TriggerMode::SameAsBus => ApicTriggerMode::Edge,
            },
            vector: 32 + legacy_irq,
        };

        apic.map(redir_tbl_index, map_info);
    }

    // Now that we've set up the IOAPIC we need to tell the firmware what we did

    match aml::AmlName::from_str("\\_PIC").and_then(|path| {
        let args = alloc::vec![aml::value::AmlValue::Integer(1)];
        acpi.aml_context
            .invoke_method(&path, aml::value::Args::from_list(args))
    }) {
        Ok(_) | Err(aml::AmlError::ValueDoesNotExist(_)) => (),

        Err(e) => panic!("Error running \\_PIC: {:?}", e),
    }
}

pub fn io_apics<'a>() -> &'a [IoApic] {
    unsafe { IOAPICS.as_ref().map_or(&[], |vector| &vector[..]) }
}

pub fn src_overrides<'a>() -> &'a [Override] {
    unsafe { SRC_OVERRIDES.as_ref().map_or(&[], |vector| &vector[..]) }
}

fn get_src_override<'a>(irq: u8) -> Option<&'a Override> {
    src_overrides().iter().find(|o| o.isa_source == irq)
}

fn find_ioapic<'a>(global_system_interrupt: u32) -> Option<&'a IoApic> {
    io_apics().iter().find(|apic| {
        global_system_interrupt >= apic.global_system_interrupt_base
            && global_system_interrupt < apic.global_system_interrupt_base + u32::from(apic.count)
    })
}
