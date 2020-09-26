#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiKind {
    Tlb = 0xf0,
    Halt = 0xfe,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiTarget {
    Current = 1,
    All = 2,
    Other = 3,
}

pub fn ipi(kind: IpiKind, target: IpiTarget) {
    use crate::devices::local_apic::local_apic_access_safe;
    
    if let Some(local_apic) = local_apic_access_safe() {
        let icr = (target as u64) << 18 | 1 << 14 | (kind as u64);
        local_apic.set_icr(icr);
    }
}