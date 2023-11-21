use crate::bus;
use crate::bus::*;
use crate::opcodes::*;

use std::fmt::Debug;

const STACK: u16 = 0x0100;
const STACK_RESET: u8 = 0xfd;

pub trait Mem {
    fn mem_read(&self, addr: u16) -> u8;

    fn mem_write(&mut self, addr: u16, data: u8);

    fn mem_read_u16(&self, pos: u16) -> u16 {
        let lo = self.mem_read(pos) as u16;
        let hi = self.mem_read(pos + 1) as u16;
        (hi << 8) | (lo as u16)
    }

    fn mem_write_u16(&mut self, pos: u16, data: u16) {
        let hi = (data >> 8) as u8;
        let lo = (data & 0xff) as u8;
        self.mem_write(pos, lo);
        self.mem_write(pos + 1, hi);
    }
}

pub struct Cpu {
    pub register_a: u8,
    pub register_x: u8,
    pub register_y: u8,

    pub status: CpuFlags,
    pub program_counter: u16,
    pub stack_pointer: u8,

    // Cpu only has 2 KiB of RAM, NEW has 64 KiB of memory
    // Program starts at 0x8000 to 0xFFFF
    bus: Bus,
}

impl Debug for Cpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let memory_slice = &self.bus.cpu_vram[..std::cmp::min(16, self.bus.cpu_vram.len())];

        f.debug_struct("Cpu")
            .field("register_a", &self.register_a)
            .field("register_x", &self.register_x)
            .field("register_y", &self.register_y)
            .field("status", &self.status)
            .field("program_counter", &self.program_counter)
            .field("memory", &memory_slice)
            .finish()
    }
}

impl Mem for Cpu {
    fn mem_read(&self, addr: u16) -> u8 {
        self.bus.mem_read(addr)
    }

    fn mem_write(&mut self, addr: u16, data: u8) {
        self.bus.mem_write(addr, data)
    }
    fn mem_read_u16(&self, pos: u16) -> u16 {
        self.bus.mem_read_u16(pos)
    }

    fn mem_write_u16(&mut self, pos: u16, data: u16) {
        self.bus.mem_write_u16(pos, data)
    }
}

impl Cpu {
    pub fn new(bus: Bus) -> Self {
        Self {
            status: CpuFlags::from_bits_truncate(0b100100),
            stack_pointer: STACK_RESET,
            register_a: 0,
            register_x: 0,
            register_y: 0,
            program_counter: 0,
            bus,
        }
    }

    fn get_operand_address(&mut self, mode: &AddressingMode) -> u16 {
        match mode {
            AddressingMode::Immediate => self.program_counter,

            AddressingMode::ZeroPage => self.mem_read(self.program_counter) as u16,

            AddressingMode::Absolute => self.mem_read_u16(self.program_counter),

            AddressingMode::ZeroPage_X => {
                let pos = self.mem_read(self.program_counter);
                pos.wrapping_add(self.register_x) as u16
            }
            AddressingMode::ZeroPage_Y => {
                let pos = self.mem_read(self.program_counter);
                pos.wrapping_add(self.register_y) as u16
            }

            AddressingMode::Absolute_X => {
                let base = self.mem_read_u16(self.program_counter);
                base.wrapping_add(self.register_x as u16)
            }
            AddressingMode::Absolute_Y => {
                let base = self.mem_read_u16(self.program_counter);
                base.wrapping_add(self.register_y as u16)
            }

            AddressingMode::Indirect_X => {
                let base = self.mem_read(self.program_counter);

                let ptr: u8 = base.wrapping_add(self.register_x);
                let lo = self.mem_read(ptr as u16);
                let hi = self.mem_read(ptr.wrapping_add(1) as u16);
                (hi as u16) << 8 | (lo as u16)
            }
            AddressingMode::Indirect_Y => {
                let base = self.mem_read(self.program_counter);

                let lo = self.mem_read(base as u16);
                let hi = self.mem_read(base.wrapping_add(1) as u16);
                let deref_base = (hi as u16) << 8 | (lo as u16);
                deref_base.wrapping_add(self.register_y as u16)
            }

            AddressingMode::NoneAddressing => {
                panic!("mode {:?} is not supported", mode);
            }
        }
    }

    /// Updates zero and negative flag based on the value passed
    fn update_zero_and_negative_flag(&mut self, target_register: u8) {
        // Zero flag
        if target_register == 0 {
            self.status.insert(CpuFlags::ZERO)
        } else {
            self.status.remove(CpuFlags::ZERO)
        }

        // Negative flag
        if target_register & 0b1000_0000 != 0 {
            self.status.insert(CpuFlags::NEGATIVE)
        } else {
            self.status.remove(CpuFlags::NEGATIVE)
        }
    }

    pub fn reset(&mut self) {
        self.register_a = 0;
        self.register_x = 0;
        self.status = CpuFlags::from_bits_truncate(0b100100);

        self.program_counter = self.mem_read_u16(0xFFFC);
    }

    pub fn load(&mut self, program: Vec<u8>) {
        for i in 0..(program.len() as u16) {
            self.mem_write(0x8600 + i, program[i as usize]);
        }
        self.mem_write_u16(0xFFFC, 0x8600);
    }

    pub fn load_and_run(&mut self, program: Vec<u8>) {
        self.load(program);
        self.reset();
        self.run();
    }

    fn stack_pop(&mut self) -> u8 {
        self.stack_pointer = self.stack_pointer.wrapping_add(1);
        self.mem_read((STACK) + self.stack_pointer as u16)
    }

    fn stack_push(&mut self, data: u8) {
        self.mem_write((STACK) + self.stack_pointer as u16, data);
        self.stack_pointer = self.stack_pointer.wrapping_sub(1)
    }

    fn stack_push_u16(&mut self, data: u16) {
        let hi = (data >> 8) as u8;
        let lo = (data & 0xff) as u8;
        self.stack_push(hi);
        self.stack_push(lo);
    }

    fn stack_pop_u16(&mut self) -> u16 {
        let lo = self.stack_pop() as u16;
        let hi = self.stack_pop() as u16;

        hi << 8 | lo
    }

    fn lda(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let value = self.mem_read(addr);

        self.register_a = value;
        self.update_zero_and_negative_flag(self.register_a);
    }

    fn ldx(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let value = self.mem_read(addr);

        self.register_x = value;
        self.update_zero_and_negative_flag(self.register_a);
    }

    fn ldy(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let value = self.mem_read(addr);

        self.register_y = value;
        self.update_zero_and_negative_flag(self.register_a);
    }

    fn sta(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        self.mem_write(addr, self.register_a);
    }

    fn stx(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        self.mem_write(addr, self.register_x);
    }

    fn sty(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        self.mem_write(addr, self.register_y);
    }

    fn tax(&mut self) {
        self.register_x = self.register_a;
        self.update_zero_and_negative_flag(self.register_x);
    }

    fn tay(&mut self) {
        self.register_y = self.register_a;
        self.update_zero_and_negative_flag(self.register_y);
    }

    fn txa(&mut self) {
        self.register_a = self.register_x;
        self.update_zero_and_negative_flag(self.register_a);
    }

    fn tya(&mut self) {
        self.register_a = self.register_y;
        self.update_zero_and_negative_flag(self.register_a);
    }

    fn inx(&mut self) {
        self.register_x = self.register_x.wrapping_add(1);
        self.update_zero_and_negative_flag(self.register_x);
    }

    fn iny(&mut self) {
        self.register_y = self.register_y.wrapping_add(1);
        self.update_zero_and_negative_flag(self.register_y);
    }

    fn dex(&mut self) {
        self.register_x = self.register_x.wrapping_sub(1);
        self.update_zero_and_negative_flag(self.register_x);
    }

    fn dey(&mut self) {
        self.register_y = self.register_y.wrapping_sub(1);
        self.update_zero_and_negative_flag(self.register_y);
    }

    fn set_register_a(&mut self, value: u8) {
        self.register_a = value;
        self.update_zero_and_negative_flag(self.register_a);
    }

    fn adc(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let value = self.mem_read(addr);
        self.add_to_register_a(value);
    }

    fn sre(&mut self, mode: &AddressingMode) {
        let data = self.lsr(mode);
        self.xor_with_register_a(data);
    }

    fn add_to_register_a(&mut self, data: u8) {
        let sum = self.register_a as u16
            + data as u16
            + (if self.status.contains(CpuFlags::CARRY) {
                1
            } else {
                0
            }) as u16;

        let carry = sum > 0xff;

        if carry {
            self.status.insert(CpuFlags::CARRY);
        } else {
            self.status.remove(CpuFlags::CARRY);
        }

        let result = sum as u8;

        if (data ^ result) & (result ^ self.register_a) & 0x80 != 0 {
            self.status.insert(CpuFlags::OVERFLOW);
        } else {
            self.status.remove(CpuFlags::OVERFLOW)
        }

        self.set_register_a(result);
    }

    fn sbc(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let data = self.mem_read(addr);
        self.add_to_register_a(((data as i8).wrapping_neg().wrapping_sub(1)) as u8);
    }

    fn php(&mut self) {
        //http://wiki.nesdev.com/w/index.php/CPU_status_flag_behavior
        let mut flags = self.status.clone();
        flags.insert(CpuFlags::BREAK);
        flags.insert(CpuFlags::BREAK2);
        self.stack_push(flags.bits());
    }

    fn plp(&mut self) {
        self.status = CpuFlags::from_bits(self.stack_pop()).unwrap();
        self.status.remove(CpuFlags::BREAK);
        self.status.insert(CpuFlags::BREAK2);
    }

    fn pla(&mut self) {
        let data = self.stack_pop();
        self.set_register_a(data);
    }

    fn and(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let data = self.mem_read(addr);
        self.set_register_a(data & self.register_a);
    }

    fn inc(&mut self, mode: &AddressingMode) -> u8 {
        let addr = self.get_operand_address(mode);
        let mut data = self.mem_read(addr);
        data = data.wrapping_add(1);
        self.mem_write(addr, data);
        self.update_zero_and_negative_flag(data);
        data
    }

    fn lsr(&mut self, mode: &AddressingMode) -> u8 {
        match mode {
            AddressingMode::NoneAddressing => {
                let mut data = self.register_a;
                if data & 1 == 1 {
                    self.status.insert(CpuFlags::CARRY);
                } else {
                    self.status.remove(CpuFlags::CARRY);
                }
                data >>= 1;
                return data;
            }
            _ => {
                let addr = self.get_operand_address(mode);
                let mut data = self.mem_read(addr);
                if data & 1 == 1 {
                    self.status.insert(CpuFlags::CARRY);
                } else {
                    self.status.remove(CpuFlags::CARRY);
                }
                data >>= 1;
                self.mem_write(addr, data);
                self.update_zero_and_negative_flag(data);
                self.set_register_a(data);
                return 0;
            }
        }
    }

    fn asl(&mut self, mode: &AddressingMode) -> u8 {
        match mode {
            AddressingMode::NoneAddressing => {
                let mut data = self.register_a;
                if data >> 7 == 1 {
                    self.status.insert(CpuFlags::CARRY)
                } else {
                    self.status.remove(CpuFlags::CARRY)
                }
                data <<= 1;
                return data;
            }
            _ => {
                let addr = self.get_operand_address(mode);
                let mut data = self.mem_read(addr);
                if data >> 7 == 1 {
                    self.status.insert(CpuFlags::CARRY)
                } else {
                    self.status.remove(CpuFlags::CARRY)
                }
                data <<= 1;
                self.mem_write(addr, data);
                self.update_zero_and_negative_flag(data);
                return 0;
            }
        }
    }

    fn rol(&mut self, mode: &AddressingMode) -> u8 {
        match mode {
            AddressingMode::NoneAddressing => {
                let mut data = self.register_a;
                let old_carry = self.status.contains(CpuFlags::CARRY);

                if data >> 7 == 1 {
                    self.status.insert(CpuFlags::CARRY);
                } else {
                    self.status.remove(CpuFlags::CARRY);
                }
                data <<= 1;
                if old_carry {
                    data |= 1;
                }
                return data;
            }
            _ => {
                let addr = self.get_operand_address(mode);
                let mut data = self.mem_read(addr);
                let old_carry = self.status.contains(CpuFlags::CARRY);

                if data >> 7 == 1 {
                    self.status.insert(CpuFlags::CARRY);
                } else {
                    self.status.remove(CpuFlags::CARRY);
                }
                data <<= 1;
                if old_carry {
                    data |= 1;
                }
                self.mem_write(addr, data);
                self.update_zero_and_negative_flag(data);
                return 0;
            }
        }
    }

    fn and_with_register_a(&mut self, data: u8) {
        self.set_register_a(data & self.register_a);
    }

    fn xor_with_register_a(&mut self, data: u8) {
        self.set_register_a(data ^ self.register_a);
    }

    fn rla(&mut self, mode: &AddressingMode) {
        let data = self.rol(mode);
        self.and_with_register_a(data);
    }

    fn rra(&mut self, mode: &AddressingMode) {
        let data = self.ror(mode);
        self.add_to_register_a(data);
    }

    fn ror(&mut self, mode: &AddressingMode) -> u8 {
        match mode {
            AddressingMode::NoneAddressing => {
                let mut data = self.register_a;
                let old_carry = self.status.contains(CpuFlags::CARRY);

                if data & 1 == 1 {
                    self.status.insert(CpuFlags::CARRY);
                } else {
                    self.status.remove(CpuFlags::CARRY);
                }
                data >>= 1;
                if old_carry {
                    data |= 0b10000000;
                }
                return data;
            }
            _ => {
                let addr = self.get_operand_address(mode);
                let mut data = self.mem_read(addr);
                let old_carry = self.status.contains(CpuFlags::CARRY);

                if data & 1 == 1 {
                    self.status.insert(CpuFlags::CARRY);
                } else {
                    self.status.remove(CpuFlags::CARRY);
                }
                data >>= 1;
                if old_carry {
                    data |= 0b10000000;
                }
                self.mem_write(addr, data);
                self.update_zero_and_negative_flag(data);
                return 0;
            }
        }
    }

    fn jsr(&mut self) {
        self.stack_push_u16(self.program_counter + 2 - 1);
        let target_address = self.mem_read_u16(self.program_counter);
        self.program_counter = target_address
    }

    fn rts(&mut self) {
        self.program_counter = self.stack_pop_u16() + 1;
    }

    // Not sure if correct lol
    fn rti(&mut self) {
        self.status = CpuFlags::from_bits(self.stack_pop()).expect("Couldn't create bitflag");
        self.program_counter = self.stack_pop_u16();
    }

    fn branch(&mut self, condition: bool) {
        if condition {
            let jump: i8 = self.mem_read(self.program_counter) as i8;
            let jump_addr = self
                .program_counter
                .wrapping_add(1)
                .wrapping_add(jump as u16);

            self.program_counter = jump_addr;
        }
    }

    fn compare(&mut self, mode: &AddressingMode, compare_with: u8) {
        let addr = self.get_operand_address(mode);
        let data = self.mem_read(addr);
        if data <= compare_with {
            self.status.insert(CpuFlags::CARRY);
        } else {
            self.status.remove(CpuFlags::CARRY);
        }

        self.update_zero_and_negative_flag(compare_with.wrapping_sub(data));
    }

    fn bit(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let data = self.mem_read(addr);
        let and = self.register_a & data;
        if and == 0 {
            self.status.insert(CpuFlags::ZERO);
        } else {
            self.status.remove(CpuFlags::ZERO);
        }

        self.status.set(CpuFlags::NEGATIVE, data & 0b10000000 > 0);
        self.status.set(CpuFlags::OVERFLOW, data & 0b01000000 > 0);
    }

    fn jmp(&mut self, mode: &AddressingMode) {
        match mode {
            AddressingMode::NoneAddressing => {
                let mem_address = self.mem_read_u16(self.program_counter);
                // let indirect_ref = self.mem_read_u16(mem_address);
                //6502 bug mode with with page boundary:
                //  if address $3000 contains $40, $30FF contains $80, and $3100 contains $50,
                // the result of JMP ($30FF) will be a transfer of control to $4080 rather than $5080 as you intended
                // i.e. the 6502 took the low byte of the address from $30FF and the high byte from $3000

                let indirect_ref = if mem_address & 0x00FF == 0x00FF {
                    let lo = self.mem_read(mem_address);
                    let hi = self.mem_read(mem_address & 0xFF00);
                    (hi as u16) << 8 | (lo as u16)
                } else {
                    self.mem_read_u16(mem_address)
                };

                self.program_counter = indirect_ref;
            }
            AddressingMode::Absolute => {
                let mem_address = self.mem_read_u16(self.program_counter);
                self.program_counter = mem_address;
            }
            _ => (),
        }
    }

    fn dec(&mut self, mode: &AddressingMode) -> u8 {
        let addr = self.get_operand_address(mode);
        let mut data = self.mem_read(addr);
        data = data.wrapping_sub(1);
        self.mem_write(addr, data);
        self.update_zero_and_negative_flag(data);
        data
    }

    fn txs(&mut self) {
        self.stack_pointer = self.register_x;
    }

    fn tsx(&mut self) {
        self.register_x = self.stack_pointer;
        self.update_zero_and_negative_flag(self.register_x);
    }

    fn eor(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let data = self.mem_read(addr);
        self.set_register_a(data ^ self.register_a);
    }

    fn ora(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let data = self.mem_read(addr);
        self.set_register_a(data | self.register_a);
    }

    fn lax(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let data = self.mem_read(addr);
        self.set_register_a(data);
        self.register_x = self.register_a;
    }

    fn sub_from_register_a(&mut self, data: u8) {
        self.add_to_register_a(((data as i8).wrapping_neg().wrapping_sub(1)) as u8);
    }

    fn isb(&mut self, mode: &AddressingMode) {
        let data = self.inc(mode);
        self.sub_from_register_a(data);
    }

    fn dcp(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        let mut data = self.mem_read(addr);
        data = data.wrapping_sub(1);
        self.mem_write(addr, data);
        // self._update_zero_and_negative_flags(data);
        if data <= self.register_a {
            self.status.insert(CpuFlags::CARRY);
        }

        self.update_zero_and_negative_flag(self.register_a.wrapping_sub(data));
    }

    fn aax(&mut self, mode: &AddressingMode) {
        let addr = self.get_operand_address(mode);
        self.mem_write(addr, self.register_x & self.register_a);
    }

    fn or_with_register_a(&mut self, data: u8) {
        self.set_register_a(data | self.register_a);
    }

    fn slo(&mut self, mode: &AddressingMode) {
        let data = self.asl(mode);
        self.or_with_register_a(data);
    }

    pub fn get_absolute_address(&self, mode: &AddressingMode, addr: u16) -> u16 {
        match mode {
            AddressingMode::ZeroPage => self.mem_read(addr) as u16,

            AddressingMode::Absolute => self.mem_read_u16(addr),

            AddressingMode::ZeroPage_X => {
                let pos = self.mem_read(addr);
                let addr = pos.wrapping_add(self.register_x) as u16;
                addr
            }
            AddressingMode::ZeroPage_Y => {
                let pos = self.mem_read(addr);
                let addr = pos.wrapping_add(self.register_y) as u16;
                addr
            }

            AddressingMode::Absolute_X => {
                let base = self.mem_read_u16(addr);
                let addr = base.wrapping_add(self.register_x as u16);
                addr
            }
            AddressingMode::Absolute_Y => {
                let base = self.mem_read_u16(addr);
                let addr = base.wrapping_add(self.register_y as u16);
                addr
            }

            AddressingMode::Indirect_X => {
                let base = self.mem_read(addr);

                let ptr: u8 = (base as u8).wrapping_add(self.register_x);
                let lo = self.mem_read(ptr as u16);
                let hi = self.mem_read(ptr.wrapping_add(1) as u16);
                (hi as u16) << 8 | (lo as u16)
            }
            AddressingMode::Indirect_Y => {
                let base = self.mem_read(addr);

                let lo = self.mem_read(base as u16);
                let hi = self.mem_read((base as u8).wrapping_add(1) as u16);
                let deref_base = (hi as u16) << 8 | (lo as u16);
                let deref = deref_base.wrapping_add(self.register_y as u16);
                deref
            }

            _ => {
                panic!("mode {:?} is not supported", mode);
            }
        }
    }

    pub fn run(&mut self) {
        self.run_with_callback(|_| {})
    }

    pub fn run_with_callback<F>(&mut self, mut callback: F)
    where
        F: FnMut(&mut Cpu),
    {
        // Fetch next execution instruction from the instruction memory
        // Decode the instruction
        // Execute the Instruction
        // Repeat the cycle

        loop {
            callback(self);
            // println!("{:?}", self);
            let opcode = self.mem_read(self.program_counter);
            self.program_counter += 1;
            let program_counter_state = self.program_counter;
            // print!("Opcode: 0x{:X}", opcode);
            let operation = OPCODES_MAP[&opcode];
            // println!(", Mnemonic: {:?}", operation.mnemonic);

            match operation.mnemonic {
                "ADC" => self.adc(&operation.mode),
                "AND" => self.and(&operation.mode),
                "ASL" => {
                    self.asl(&operation.mode);
                }
                "BCC" => self.branch(!self.status.contains(CpuFlags::CARRY)),
                "BCS" => self.branch(self.status.contains(CpuFlags::CARRY)),
                "BEQ" => self.branch(self.status.contains(CpuFlags::ZERO)),
                "BMI" => self.branch(self.status.contains(CpuFlags::NEGATIVE)),
                "BNE" => self.branch(!self.status.contains(CpuFlags::ZERO)),
                "BPL" => self.branch(!self.status.contains(CpuFlags::NEGATIVE)),
                "BRK" => return,
                "BVC" => self.branch(!self.status.contains(CpuFlags::OVERFLOW)),
                "BVS" => self.branch(self.status.contains(CpuFlags::OVERFLOW)),
                "CLC" => self.status.remove(CpuFlags::CARRY),
                "CLD" => self.status.remove(CpuFlags::DECIMAL_MODE),
                "CLI" => self.status.remove(CpuFlags::INTERRUPT_DISABLE),
                "CLV" => self.status.remove(CpuFlags::OVERFLOW),
                "CMP" => self.compare(&operation.mode, self.register_a),
                "CPX" => self.compare(&operation.mode, self.register_x),
                "CPY" => self.compare(&operation.mode, self.register_y),
                "DEX" => self.dex(),
                "INX" => self.inx(),
                "INY" => self.iny(),
                "JSR" => self.jsr(),
                "LDA" => self.lda(&operation.mode),
                "LDX" => self.ldx(&operation.mode),
                "PHA" => self.stack_push(self.register_a),
                "PHP" => self.php(),
                "PLA" => self.pla(),
                "PLP" => self.plp(),
                "ROL" => {
                    self.rol(&operation.mode);
                }
                "ROR" => {
                    self.ror(&operation.mode);
                }
                "RTS" => self.rts(),
                "SBC" => self.sbc(&operation.mode),
                "SEC" => self.status.insert(CpuFlags::CARRY),
                "SED" => self.status.insert(CpuFlags::DECIMAL_MODE),
                "SEI" => self.status.insert(CpuFlags::INTERRUPT_DISABLE),
                "STA" => self.sta(&operation.mode),
                "TAX" => self.tax(),
                "TXA" => self.txa(),
                "LSR" => {
                    self.lsr(&operation.mode);
                }
                "INC" => {
                    self.inc(&operation.mode);
                }
                "BIT" => self.bit(&operation.mode),
                "LDY" => self.ldy(&operation.mode),
                "NOP" => (),
                "JMP" => self.jmp(&operation.mode),
                "DEC" => {
                    self.dec(&operation.mode);
                }
                "TXS" => self.txs(),
                "TSX" => self.tsx(),
                "STX" => self.stx(&operation.mode),
                "STY" => self.sty(&operation.mode),
                "ORA" => self.ora(&operation.mode),
                "EOR" => self.eor(&operation.mode),
                "DEY" => self.dey(),
                "TAY" => self.tay(),
                "TYA" => self.tya(),
                "RTI" => self.rti(),
                "DOP" => (),
                "TOP" => (),
                "LAX" => self.lax(&operation.mode),
                "AAX" => self.aax(&operation.mode),
                "DCP" => self.dcp(&operation.mode),
                "ISB" => self.isb(&operation.mode),
                "SLO" => self.slo(&operation.mode),
                "RLA" => self.rla(&operation.mode),
                "SRE" => self.sre(&operation.mode),
                "RRA" => self.rra(&operation.mode),
                _ => todo!(),
            }

            if program_counter_state == self.program_counter {
                self.program_counter += (operation.len - 1) as u16;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::rom::Rom;

    use super::*;

    // #[test]
    // fn test_0xa9_lda_immediate_load_data() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load_and_run(vec![0xa9, 0x05, 0x00]);
    //     assert_eq!(cpu.register_a, 0x05);
    //     assert!(cpu.status.bits() & 0b0000_0010 == 0b00);
    //     assert!(cpu.status.bits() & 0b1000_0000 == 0);
    // }

    // #[test]
    // fn test_0xa9_lda_zero_flag() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load_and_run(vec![0xa9, 0x00]);
    //     assert!(cpu.status.bits() & 0b0000_0010 == 0b10);
    // }

    // #[test]
    // fn test_0xaa_tax_move_a_to_x() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load(vec![0xaa, 0x00]);
    //     cpu.register_a = 10;
    //     cpu.program_counter = cpu.mem_read_u16(0xFFFC);
    //     cpu.run();
    //     assert_eq!(cpu.register_x, 10)
    // }

    // #[test]
    // fn test_5_ops_working_together() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load_and_run(vec![0xa9, 0xc0, 0xaa, 0xe8]);
    //     assert_eq!(cpu.register_x, 0xc1)
    // }

    // #[test]
    // fn test_inx_overflow() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load(vec![0xe8, 0xe8, 0x00]);
    //     cpu.register_x = 0xff;
    //     cpu.program_counter = cpu.mem_read_u16(0xFFFC);
    //     cpu.run();
    //     assert_eq!(cpu.register_x, 1)
    // }

    // #[test]
    // fn test_lda_from_memory() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.mem_write(0x10, 0x55);
    //     cpu.load_and_run(vec![0xa5, 0x10, 0x00]);
    //     assert_eq!(cpu.register_a, 0x55);
    // }

    // #[test]
    // fn test_sta_zero_page() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));

    //     // Load the program into memory, including the STA ZeroPage instruction
    //     cpu.load_and_run(vec![
    //         0xa9, 0x42, // LDA Immediate, load value 0x42 into A
    //         0x85, 0x10,
    //     ]); // STA ZeroPage, store A into memory at address 0x10

    //     // Check if the value in memory at address 0x10 is equal to the value in register A
    //     assert_eq!(cpu.mem_read(0x10), cpu.register_a);
    // }

    // #[test]
    // fn test_sta_zero_page_x() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));

    //     // Load the program into memory, including the STA ZeroPage_X instruction
    //     cpu.load_and_run(vec![
    //         0xa9, 0x42, // LDA Immediate, load value 0x42 into A
    //         0xa2, 0x05, // LDX Immediate, load value 0x05 into X
    //         0x95, 0x10,
    //     ]); // STA ZeroPage_X, store A into memory at address 0x10 + X

    //     // Calculate the expected address based on ZeroPage_X addressing mode
    //     let expected_address = (0x10 + cpu.register_x as u16) & 0xFF;

    //     // Check if the value in memory at the expected address is equal to the value in register A
    //     assert_eq!(cpu.mem_read(expected_address), cpu.register_a);
    // }

    // #[test]
    // fn test_adc_carry_flag() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load_and_run(vec![0xa9, 0xff, 0x69, 0x02]);
    //     assert_eq!(cpu.register_a, 0x01);
    //     assert!(!cpu.status.contains(CpuFlags::ZERO));
    //     assert!(!cpu.status.contains(CpuFlags::NEGATIVE));
    //     assert!(cpu.status.contains(CpuFlags::CARRY));
    //     assert!(!cpu.status.contains(CpuFlags::OVERFLOW));
    // }

    // #[test]
    // fn test_adc_overflow_flag() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load_and_run(vec![0xa9, 0x7f, 0x69, 0x01]);
    //     assert_eq!(cpu.register_a, 0x80);
    //     assert!(!cpu.status.contains(CpuFlags::ZERO));
    //     assert!(cpu.status.contains(CpuFlags::NEGATIVE));
    //     assert!(!cpu.status.contains(CpuFlags::CARRY));
    //     assert!(cpu.status.contains(CpuFlags::OVERFLOW));
    // }

    // #[test]
    // fn test_sbc_immediate() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.load_and_run(vec![0xa9, 0x05, 0xe9, 0x02]);
    //     assert_eq!(cpu.register_a, 0x03);
    //     assert!(!cpu.status.contains(CpuFlags::ZERO));
    //     assert!(!cpu.status.contains(CpuFlags::NEGATIVE));
    //     assert!(!cpu.status.contains(CpuFlags::CARRY));
    //     assert!(!cpu.status.contains(CpuFlags::OVERFLOW));
    // }

    // #[test]
    // fn test_sbc_zero_page() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     // LDA 0x05
    //     // store reg_a at
    //     cpu.load_and_run(vec![0xa9, 0x05, 0x85, 0x10, 0xe5, 0x10]);
    //     assert_eq!(cpu.register_a, 0x00);
    //     assert!(cpu.status.contains(CpuFlags::ZERO));
    //     assert!(!cpu.status.contains(CpuFlags::NEGATIVE));
    //     assert!(!cpu.status.contains(CpuFlags::CARRY));
    //     assert!(!cpu.status.contains(CpuFlags::OVERFLOW));
    // }

    // #[test]
    // fn test_php() {
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));
    //     cpu.status = CpuFlags::ZERO | CpuFlags::NEGATIVE;

    //     // Call the php function to push the status to the stack
    //     cpu.php();

    //     // Pop the value from the stack and check if the flags are set correctly
    //     let popped_flags = cpu.stack_pop();
    //     let expected_flags =
    //         CpuFlags::ZERO | CpuFlags::NEGATIVE | CpuFlags::BREAK | CpuFlags::BREAK2;

    //     assert_eq!(popped_flags, expected_flags.bits());
    // }

    // #[test]
    // fn test_plp() {
    //     // Create a Cpu instance
    //     let mut cpu = Cpu::new(Bus::new(Rom::new(vec![0_u8; 10])));

    //     // Set some flags, including BREAK and BREAK2
    //     let mut flags = CpuFlags::empty();
    //     flags.insert(CpuFlags::CARRY);
    //     flags.insert(CpuFlags::ZERO);

    //     cpu.status = flags.clone();

    //     // Push the flags onto the stack
    //     cpu.php();

    //     // Clear the flags before calling plp
    //     cpu.status = CpuFlags::empty();

    //     // Call the plp method to pop the flags from the stack
    //     cpu.plp();

    //     // Check if the flags in the status field match the original flags
    //     assert_eq!(cpu.status.bits(), flags.bits());
    // }
}
