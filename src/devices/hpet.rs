use crate::acpi::ACPI;
use crate::init_mutex::InitMutex;
use crate::paging::{self, Region};

static LEG_RT_CNF: u64 = 2;
static ENABLE_CNF: u64 = 1;

static TN_VAL_SET_CNF: u64 = 0x40;
static TN_TYPE_CNF: u64 = 0x08;
static TN_INT_ENB_CNF: u64 = 0x04;

static CAPABILITY_OFFSET: u16 = 0x00;
static GENERAL_CONFIG_OFFSET: u16 = 0x10;
// static GENERAL_INTERRUPT_OFFSET: usize = 0x20;
// static MAIN_COUNTER_OFFSET: usize = 0xF0;
// static NUM_TIMER_CAP_MASK: u64 = 0x0f00;
static LEG_RT_CAP: u64 = 0x8000;
static T0_CONFIG_CAPABILITY_OFFSET: u16 = 0x100;
static T0_COMPARATOR_OFFSET: u16 = 0x108;

static PER_INT_CAP: u64 = 0x10;

struct HpetAccess {
    mapping: Region,
}

impl HpetAccess {
    pub unsafe fn new(
        _event_timer_block_id: u32,
        base_address: usize,
        _hpet_number: u8,
        _clock_tick_unit: u16,
    ) -> Option<Self> {
        paging::map_physical_memory(base_address, 1024, paging::PhysicalMappingFlags::UNCACHED)
            .map(|mapping| Self { mapping })
            .ok()
    }

    pub unsafe fn read(&self, register: u16) -> u64 {
        use core::intrinsics::volatile_load;
        volatile_load(self.mapping.as_ptr_offset(register.into()))
    }

    pub unsafe fn write(&mut self, register: u16, value: u64) {
        use core::intrinsics::volatile_store;
        volatile_store(self.mapping.as_mut_ptr_offset(register.into()), value);
    }

    pub fn current(&self) -> u64 {
        unsafe { self.read(0xf0) }
    }
}

pub struct Hpet {
    access: HpetAccess,
}

impl Hpet {
    unsafe fn new(access: HpetAccess) -> Self {
        let mut ret = Self { access };

        let capability = ret.access.read(CAPABILITY_OFFSET);
        if capability & LEG_RT_CAP == 0 {
            panic!("HPET cannot perform legacy replacement")
        }

        let counter_clk_period_fs = capability >> 32;
        let desired_fs_period: u64 = 2_250_286 * 1_000_000;

        let clk_periods_per_kernel_tick: u64 = desired_fs_period / counter_clk_period_fs;

        let t0_capabilities = ret.access.read(T0_CONFIG_CAPABILITY_OFFSET);
        if t0_capabilities & PER_INT_CAP == 0 {
            panic!("HPET timer 0 does not support periodic mode");
        }

        let t0_config_word: u64 = TN_VAL_SET_CNF | TN_TYPE_CNF | TN_INT_ENB_CNF;
        ret.access
            .write(T0_CONFIG_CAPABILITY_OFFSET, t0_config_word);
        ret.access.write(
            T0_COMPARATOR_OFFSET,
            ret.access.current() + clk_periods_per_kernel_tick,
        );
        // set accumulator value
        ret.access
            .write(T0_COMPARATOR_OFFSET, clk_periods_per_kernel_tick);
        // set interval

        let enable_word: u64 = ret.access.read(GENERAL_CONFIG_OFFSET) | LEG_RT_CNF | ENABLE_CNF;
        ret.access.write(GENERAL_CONFIG_OFFSET, enable_word);
        // Enable interrupts from the HPET

        ret
    }
}

pub static HPET: InitMutex<Hpet> = InitMutex::new();

pub unsafe fn init() {
    let mut acpi_lock = ACPI.lock();
    let acpi = acpi_lock.as_mut().unwrap();

    HPET.init(
        acpi.acpi_context
            .hpet
            .as_ref()
            .and_then(|hpet| {
                HpetAccess::new(
                    hpet.event_timer_block_id,
                    hpet.base_address,
                    hpet.hpet_number,
                    hpet.clock_tick_unit,
                )
            })
            .map(|access| Hpet::new(access))
            .expect("Failed to locate HPET"),
    );
}
