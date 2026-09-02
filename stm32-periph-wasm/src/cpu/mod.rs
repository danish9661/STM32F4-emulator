pub mod regs;
pub mod mem;
mod thumb;
pub use regs::Regs;
pub use mem::Memory;
use crate::system::WasmSystem;
pub struct Cpu { pub regs: Regs, pub cycles: u64 }
impl Cpu {
    pub fn new(sp: u32, pc: u32) -> Self { Self { regs: Regs::new(sp, pc), cycles: 0 } }
    pub fn run(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, budget: u32) -> (u32,bool) {
        let mut done=0;
        while done<budget {
            let pc=self.regs.r[15]&!1;
            let op=mem.read16(pc);
            let l=thumb::len(op);
            let ok=if l==2 { thumb::exec16(self, sys, mem, op, pc) } else { let o2=mem.read16(pc+2); thumb::exec32(self, sys, mem, op, o2, pc) };
            if !ok { return (done,true); }
            done+=1; self.cycles+=1;
            if sys.p.nvic.borrow().has_pending() { break; }
        }
        (done,false)
    }
}
