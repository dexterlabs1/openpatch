#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce the numerical compatibility kernel with a user-supplied Rack SDK.

This optional offline design tool is never called by Cargo/app builds. No SDK
source or binaries are distributed here. The output is computed numerical data,
not an embedded Rack/Fundamental wave asset or copied program text. Rack itself
is GPLv3+; see https://github.com/VCVRack/Rack/blob/v2/LICENSE.md .
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[2]
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk',type=Path,required=True)
    parser.add_argument('--output',type=Path,default=ROOT/'crates/openpatch-core/src/wavetable_minblep.rs')
    parser.add_argument('--metadata',type=Path,required=True)
    args=parser.parse_args();sdk=args.sdk.resolve()
    with tempfile.TemporaryDirectory(prefix='openpatch-kernel-') as temporary:
        binary=Path(temporary)/'design'
        macsdk=Path(subprocess.check_output(['xcrun','--show-sdk-path'],text=True).strip())
        source=Path(__file__).with_name('sdk_coefficients.cpp')
        command=['c++','-std=c++17','-O2','-ffp-contract=off','-isysroot',str(macsdk),'-isystem',str(macsdk/'usr/include/c++/v1'),'-I'+str(sdk/'include'),'-I'+str(sdk/'dep/include'),str(source),'-L'+str(sdk),'-lRack','-o',str(binary)]
        subprocess.run(command,check=True)
        env=os.environ.copy();env['DYLD_LIBRARY_PATH']=str(sdk)
        result=subprocess.run([str(binary)],env=env,check=True,capture_output=True)
        assert len(result.stdout)==2048
        raw=result.stdout;bits=struct.unpack('<512I',raw)+(0x3f800000,)
        metadata={'source':'Rack SDK numerical design API; no SDK source copied into production','api':'rack::dsp::minBlepImpulse(16,16)','sdk_version':result.stderr.decode().strip(),'sdk_library_sha256':digest(sdk/'libRack.dylib'),'sdk_minblep_header_sha256':digest(sdk/'include/dsp/minblep.hpp'),'adapter_sha256':digest(source),'platform':os.uname().machine,'command':command,'computed_coefficients':512,'appended_endpoint':1.0,'computed_f32le_sha256':hashlib.sha256(raw).hexdigest()}
        args.metadata.write_text(json.dumps(metadata,indent=2)+'\n')
        body='\n'.join('    '+', '.join(f'f32::from_bits(0x{v:08x})' for v in bits[i:i+4])+',' for i in range(0,len(bits),4))
        args.output.write_text('//! Numerical compatibility coefficients computed offline by Rack SDK minBlepImpulse(16,16).\n//! No Rack/Fundamental program text is incorporated; the SDK is not a runtime dependency.\n//! Reproduction/provenance: scripts/wavetable_kernel/README.md. Endpoint1 appended locally.\n//! Computed f32le SHA256: '+metadata['computed_f32le_sha256']+'\n\npub(crate) const SYNC_BLEP: [f32; 513] = [\n'+body+'\n];\n')
        print(json.dumps({k:metadata[k] for k in ['sdk_version','computed_f32le_sha256','sdk_library_sha256']}))
if __name__=='__main__':main()
