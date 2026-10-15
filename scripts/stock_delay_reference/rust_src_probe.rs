// SPDX-License-Identifier: MIT
#![allow(dead_code)]
use std::io::{self,Read,Write};
#[path=__SINC_SOURCE__]mod sinc;
fn read_u32(input:&mut impl Read)->Option<u32>{let mut bytes=[0;4];match input.read(&mut bytes[..1]){Ok(0)=>None,Ok(1)=>{input.read_exact(&mut bytes[1..]).unwrap();Some(u32::from_le_bytes(bytes))},_=>panic!("read failure")}}
fn main(){
    let mut input=io::BufReader::new(io::stdin().lock());let mut output=io::BufWriter::new(io::stdout().lock());let mut state=sinc::MonoSinc::new();
    while let Some(n)=read_u32(&mut input){let capacity=read_u32(&mut input).unwrap()as usize;let mut bytes=[0;8];input.read_exact(&mut bytes).unwrap();let ratio=f64::from_le_bytes(bytes);assert!(n<=16&&capacity<=16);
        let mut values=[0.;16];for value in &mut values[..n as usize]{*value=f32::from_bits(read_u32(&mut input).unwrap());}
        let mut result=[0.;16];let(used,generated)=state.process(&values[..n as usize],&mut result[..capacity],ratio);
        assert!(used<=n as usize&&generated<=capacity);output.write_all(&(used as u32).to_le_bytes()).unwrap();output.write_all(&(generated as u32).to_le_bytes()).unwrap();
        for value in &result[..generated]{assert!(value.is_finite());output.write_all(&value.to_le_bytes()).unwrap();}
    }
}
