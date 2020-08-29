SECTIONS
{
    . = 0xFFFF800080000000;

    __kernel_start = .;
    __kernel_rodata_start = .;
    .rodata : ALIGN(0x1000)
    {
        *(.rodata*)
    }

    __kernel_rodata_end = .;
    
    __kernel_text_start = .;

    .text : ALIGN(0x1000) {
        *(.text*)
    } = 0x90909090

    __kernel_text_start = .;

    .data..percpu : ALIGN(0x1000)
    {
        __kernel_per_cpu_start = .;
        *(.data..percpu)
        __kernel_per_cpu_end = .;
    }

    __kernel_data_start = .;

    .data : ALIGN(0x1000)
    {
        *(.data*)
    }

    __kernel_data_end = .;

    .got : ALIGN(0x1000)
    {
        *(.got)
    }

    .bss : ALIGN(0x1000)
    {
        *(.bss)
    }

    . = ALIGN(0x1000);
    __kernel_end = .;
}