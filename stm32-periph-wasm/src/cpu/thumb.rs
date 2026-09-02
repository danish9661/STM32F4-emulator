use super::{mem::Memory, Cpu};
use crate::system::WasmSystem;
pub fn len(op: u16) -> usize { let t=op>>11; if t==0b11101||t==0b11110||t==0b11111 {4} else {2} }
#[inline] fn sx(v:u32,b:u32)->u32{ let s=32-b; ((v as i32)<<s>>s) as u32 }
pub fn exec16(cpu:&mut Cpu,_sys:&WasmSystem,mem:&mut dyn Memory,op:u16,pc:u32)->bool{
    if op==0xBF00||op==0xBF30||op==0xBF20{ cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xFF00)==0x4600{ let rd=((op&7)|((op>>4)&8))as usize; let rm=((op>>3)&0xF)as usize; cpu.regs.r[rd]=cpu.regs.r[rm]; cpu.regs.r[15]=if rd==15{cpu.regs.r[rd]|1}else{pc.wrapping_add(2)|1}; return true; }
    if (op&0xFF87)==0x4700{ let rm=((op>>3)&0xF)as usize; let t=cpu.regs.r[rm]; cpu.regs.r[15]=if t&1==1{t}else{t&!1|1}; return true; }
    if op==0x4770{ cpu.regs.r[15]=cpu.regs.r[14]|1; return true; }
    if (op&0xFF00)==0xB400{ let mut sp=cpu.regs.r[13]; let list=op&0x1FF; let cnt=list.count_ones(); sp=sp.wrapping_sub(cnt*4); let mut a=sp; for i in 0..8{ if (list>>i)&1==1{mem.write32(a,cpu.regs.r[i as usize]);a+=4;}} if (list>>8)&1==1{mem.write32(a,cpu.regs.r[14]);} cpu.regs.r[13]=sp; cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xFF00)==0xBC00{ let mut sp=cpu.regs.r[13]; let list=op&0x1FF; for i in 0..8{ if (list>>i)&1==1{cpu.regs.r[i as usize]=mem.read32(sp); sp+=4;}} if (list>>8)&1==1{cpu.regs.r[15]=mem.read32(sp)|1;sp+=4;}else{cpu.regs.r[15]=pc.wrapping_add(2)|1;} cpu.regs.r[13]=sp; return true; }
    if (op&0xF800)==0x4800{ let rt=((op>>8)&7)as usize; let imm=(op&0xFF)as u32*4; let base=(pc+4)&!3; cpu.regs.r[rt]=mem.read32(base.wrapping_add(imm)); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0xA000{ let rd=((op>>8)&7)as usize; let imm=(op&0xFF)as u32*4; cpu.regs.r[rd]=(pc+4&!3).wrapping_add(imm); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x2000{ let rd=((op>>8)&7)as usize; cpu.regs.r[rd]=(op&0xFF)as u32; cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x3000{ let rd=((op>>8)&7)as usize; cpu.regs.r[rd]=cpu.regs.r[rd].wrapping_add((op&0xFF)as u32); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x3800{ let rd=((op>>8)&7)as usize; cpu.regs.r[rd]=cpu.regs.r[rd].wrapping_sub((op&0xFF)as u32); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x2800{ let rn=((op>>8)&7)as usize; let _=cpu.regs.r[rn].wrapping_sub((op&0xFF)as u32); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x6800{ let rt=(op&7)as usize; let rn=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32*4; cpu.regs.r[rt]=mem.read32(cpu.regs.r[rn].wrapping_add(imm)); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x6000{ let rt=(op&7)as usize; let rn=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32*4; mem.write32(cpu.regs.r[rn].wrapping_add(imm),cpu.regs.r[rt]); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x7800{ let rt=(op&7)as usize; let rn=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32; cpu.regs.r[rt]=mem.read8(cpu.regs.r[rn].wrapping_add(imm))as u32; cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x7000{ let rt=(op&7)as usize; let rn=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32; mem.write8(cpu.regs.r[rn].wrapping_add(imm),(cpu.regs.r[rt]&0xFF)as u8); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x8000{ let rt=(op&7)as usize; let rn=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32*2; mem.write16(cpu.regs.r[rn].wrapping_add(imm),(cpu.regs.r[rt]&0xFFFF)as u16); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x8800{ let rt=(op&7)as usize; let rn=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32*2; cpu.regs.r[rt]=mem.read16(cpu.regs.r[rn].wrapping_add(imm))as u32; cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xFE00)==0x1C00{ let rd=(op&7)as usize; let rs=((op>>3)&7)as usize; let rn=((op>>6)&7)as usize; cpu.regs.r[rd]=cpu.regs.r[rs].wrapping_add(cpu.regs.r[rn]); cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x0000{ let rd=(op&7)as usize; let rs=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32; cpu.regs.r[rd]=if imm==0{cpu.regs.r[rs]}else{cpu.regs.r[rs]<<imm}; cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF800)==0x0800{ let rd=(op&7)as usize; let rs=((op>>3)&7)as usize; let imm=((op>>6)&0x1F)as u32; let imm=if imm==0{32}else{imm}; cpu.regs.r[rd]=cpu.regs.r[rs]>>imm; cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xFC00)==0x4000{ let o=(op>>6)&0xF; let rs=((op>>3)&7)as usize; let rd=(op&7)as usize; match o{0=>cpu.regs.r[rd]&=cpu.regs.r[rs],1=>cpu.regs.r[rd]^=cpu.regs.r[rs],2=>cpu.regs.r[rd]=cpu.regs.r[rd].wrapping_shl(cpu.regs.r[rs]&0x1F),3=>cpu.regs.r[rd]=cpu.regs.r[rd].wrapping_shr(cpu.regs.r[rs]&0x1F),10=>cpu.regs.r[rd]=cpu.regs.r[rd].wrapping_add(cpu.regs.r[rs]),12=>cpu.regs.r[rd]&=cpu.regs.r[rs],_=>{}} cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xFF00)==0xB000{ let imm=(op&0x7F)as u32*4; if op&0x80==0{cpu.regs.r[13]=cpu.regs.r[13].wrapping_add(imm);}else{cpu.regs.r[13]=cpu.regs.r[13].wrapping_sub(imm);} cpu.regs.r[15]=pc.wrapping_add(2)|1; return true; }
    if (op&0xF000)==0xD000 && (op&0x0F00)!=0x0E00{ let imm=sx(((op&0xFF)as u32)<<1,9); cpu.regs.r[15]=pc.wrapping_add(2).wrapping_add(imm)|1; return true; }
    if (op&0xF800)==0xE000{ let imm=sx(((op&0x7FF)as u32)<<1,12); cpu.regs.r[15]=pc.wrapping_add(2).wrapping_add(imm)|1; return true; }
    if (op&0xF500)==0xB100{ let imm=(((op>>3)&0x1F)as u32*2)|(((op>>2)&1)as u32*64); let rn=(op&7)as usize; let taken=if (op&0x800)==0{cpu.regs.r[rn]==0}else{cpu.regs.r[rn]!=0}; if taken{cpu.regs.r[15]=pc.wrapping_add(2).wrapping_add(imm)|1;}else{cpu.regs.r[15]=pc.wrapping_add(2)|1;} return true; }
    false
}
pub fn exec32(cpu:&mut Cpu,_sys:&WasmSystem,mem:&mut dyn Memory,op1:u16,op2:u16,pc:u32)->bool{
    if (op1&0xF800)==0xF000 && (op2&0xD000)==0xD000{
        let s=((op1>>10)&1)as u32; let imm10=(op1&0x3FF)as u32; let imm11=(op2&0x7FF)as u32; let j1=(op2>>13)&1; let j2=(op2>>11)&1;
        let i1=(j1 as u32 ^ s ^1); let i2=(j2 as u32 ^ s ^1);
        let off=(s<<24)|(i1<<23)|(i2<<22)|(imm10<<12)|(imm11<<1); let off=sx(off,25);
        cpu.regs.r[14]=(pc+4)|1; cpu.regs.r[15]=pc.wrapping_add(4).wrapping_add(off)|1; return true;
    }
    // MOVW 0xF240 — movw rd, #imm16
    if (op1&0xFBF0)==0xF240 {
        let rd=((op2>>8)&0xF)as usize;
        let i = ((op1>>10)&1) as u32;
        let imm4 = (op1 & 0x000F) as u32;
        let imm3 = ((op2>>12)&0x7) as u32;
        let imm8 = (op2 & 0x00FF) as u32;
        let imm16 = (i<<11) | (imm4<<12) | (imm3<<8) | imm8;
        cpu.regs.r[rd]= imm16;
        cpu.regs.r[15]=pc.wrapping_add(4)|1; return true;
    }
    // MOVT 0xF2C0 — movt rd, #imm16
    if (op1&0xFBF0)==0xF2C0 {
        let rd=((op2>>8)&0xF)as usize;
        let i = ((op1>>10)&1) as u32;
        let imm4 = (op1 & 0x000F) as u32;
        let imm3 = ((op2>>12)&0x7) as u32;
        let imm8 = (op2 & 0x00FF) as u32;
        let imm16 = (i<<11) | (imm4<<12) | (imm3<<8) | imm8;
        cpu.regs.r[rd]= (cpu.regs.r[rd] & 0x0000FFFF) | (imm16<<16);
        cpu.regs.r[15]=pc.wrapping_add(4)|1; return true;
    }
    // LDR.W / STR.W 0xF8xx
    if (op1&0xFF70)==0xF8D0 || (op1&0xFF70)==0xF850 {
        let rt=((op2>>8)&0xF)as usize; let rn=((op1>>0)&0xF)as usize;
        let imm12=op2&0xFFF;
        let addr=cpu.regs.r[rn].wrapping_add(imm12 as u32);
        if (op1&0x0010)!=0 { cpu.regs.r[rt]=mem.read32(addr); } else { mem.write32(addr,cpu.regs.r[rt]); }
        // Handle writeback if needed (not for now, just advance)
        cpu.regs.r[15]=pc.wrapping_add(4)|1; return true;
    }
    // Fallback: advance
    cpu.regs.r[15]=pc.wrapping_add(4)|1; true
}
