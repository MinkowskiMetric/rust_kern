pub mod io_apic;
pub mod local_apic;

pub unsafe fn init_bsp() {
    local_apic::init_bsp();
    io_apic::init();
}
