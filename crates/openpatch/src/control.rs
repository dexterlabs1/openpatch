//! Control-thread operations, shared by transports. Never called by the DSP loop.
use openpatch_core::{catalog, render, write_wav, Cable, Node, Patch};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs::{self, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, time::Instant};

pub struct Application {
    patch: Patch,
    revision: u64,
    workspace: PathBuf,
}

fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name).and_then(Value::as_str).ok_or_else(|| format!("{name} must be a string"))
}

impl Application {
    pub fn new(workspace: &Path) -> Result<Self, String> {
        fs::create_dir_all(workspace).map_err(|e| e.to_string())?;
        Ok(Self { patch: Patch::default(), revision: 0, workspace: workspace.canonicalize().map_err(|e| e.to_string())? })
    }

    fn snapshot(&self) -> Value { json!({"revision":self.revision,"patch":self.patch}) }

    fn file(&self, filename: &str, extension: &str) -> Result<PathBuf, String> {
        if filename.is_empty() || filename.len() > 120 || !filename.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || filename.starts_with('.') || !filename.ends_with(extension) {
            return Err(format!("filename must be a simple ASCII name ending with {extension}; no directories"));
        }
        let path = self.workspace.join(filename);
        // Reject symlinks for reads as well as writes. Writes additionally use create_new.
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() || !meta.is_file() { return Err("file must be a regular file, not a symlink".into()); }
        }
        Ok(path)
    }

    pub fn execute(&mut self, name: &str, args: Value) -> Result<Value, String> {
        if !args.is_object() { return Err("arguments must be an object".into()); }
        let schemas=tools();
        let schema=schemas.as_array().unwrap().iter().find(|s|s["name"]==name).ok_or_else(||format!("unknown tool: {name}"))?;
        let allowed=schema["inputSchema"]["properties"].as_object().unwrap();
        for key in args.as_object().unwrap().keys() {
            if !allowed.contains_key(key) {return Err(format!("unknown argument: {key}"));}
        }
        match name {
            "catalog" => return Ok(catalog()),
            "inspect_patch" => return Ok(self.snapshot()),
            "save_patch" => {
                let path = self.file(string(&args,"filename")?, ".json")?;
                let bytes = serde_json::to_vec_pretty(&self.patch).map_err(|e|e.to_string())?;
                let mut f = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|e.to_string())?;
                f.write_all(&bytes).map_err(|e|e.to_string())?;
                return Ok(json!({"filename":args["filename"],"revision":self.revision}));
            }
            "render_audio" => {
                let seconds = args.get("seconds").and_then(Value::as_f64).ok_or("seconds must be a number")?;
                if !seconds.is_finite() || !(0.001..=30.0).contains(&seconds) {return Err("seconds must be 0.001..30".into());}
                let path = self.file(string(&args,"filename")?, ".wav")?;
                if path.exists() { return Err("destination already exists".into()); }
                let samples = render(&self.patch,seconds)?;
                let peak = samples.iter().fold(0.0_f32, |p,s|p.max(s.abs()));
                let rms = (samples.iter().map(|s|(*s as f64).powi(2)).sum::<f64>() / samples.len().max(1) as f64).sqrt();
                write_wav(&path,&samples,self.patch.sample_rate).map_err(|e|e.to_string())?;
                return Ok(json!({"filename":args["filename"],"frames":samples.len(),"sample_rate":self.patch.sample_rate,"peak":peak,"rms":rms,"revision":self.revision,"state":"rendered from reset"}));
            }
            "benchmark" => {
                let seconds = match args.get("seconds") {None=>1.0,Some(v)=>v.as_f64().ok_or("seconds must be a number")?};
                if !seconds.is_finite() || !(0.01..=5.0).contains(&seconds) { return Err("benchmark seconds must be 0.01..5".into()); }
                let mut engine = self.patch.compile()?;
                let mut block = [0.0;128];
                for _ in 0..16 {engine.process_block(&mut block);}
                let frames = (seconds * self.patch.sample_rate as f64).round() as usize;
                let start = Instant::now();
                let mut remaining=frames;
                let mut checksum=0.0_f64;
                while remaining > 0 {
                    let n=remaining.min(block.len());
                    engine.process_block(&mut block[..n]);
                    checksum += std::hint::black_box(&block[..n]).iter().map(|s| *s as f64).sum::<f64>();
                    remaining-=n;
                }
                let elapsed=start.elapsed().as_secs_f64().max(1e-9);
                return Ok(json!({"frames":frames,"elapsed_seconds":elapsed,"frames_per_second":frames as f64/elapsed,"realtime_factor":frames as f64/self.patch.sample_rate as f64/elapsed,"checksum":checksum,"sample_rate":self.patch.sample_rate,"block_size":128,"nodes":self.patch.nodes.len(),"arch":std::env::consts::ARCH,"os":std::env::consts::OS,"debug_build":cfg!(debug_assertions),"comparison":"OpenPatch only; not a VCV Rack comparison"}));
            }
            _ => {}
        }
        if let Some(expected) = args.get("expected_revision") {
            let expected=expected.as_u64().ok_or("expected_revision must be a nonnegative integer")?;
            if expected != self.revision {return Err(format!("revision conflict: expected {expected}, current {}",self.revision));}
        }
        let mut candidate=self.patch.clone();
        match name {
            "replace_patch" => {candidate=serde_json::from_value(args.get("patch").ok_or("patch required")?.clone()).map_err(|e|e.to_string())?;}
            "load_patch" => {
                let path=self.file(string(&args,"filename")?,".json")?;
                let f=fs::File::open(path).map_err(|e|e.to_string())?;
                let mut data=Vec::new();
                f.take(1_048_577).read_to_end(&mut data).map_err(|e|e.to_string())?;
                if data.len()>1_048_576 {return Err("patch file exceeds 1 MiB".into());}
                candidate=serde_json::from_slice(&data).map_err(|e|e.to_string())?;
            }
            "add_module" => {
                let params:BTreeMap<String,f32>=serde_json::from_value(args.get("params").cloned().unwrap_or(json!({}))).map_err(|e|e.to_string())?;
                candidate.nodes.push(Node{id:string(&args,"id")?.into(),kind:string(&args,"kind")?.into(),params});
            }
            "remove_module" => {
                let id=string(&args,"id")?;
                if !candidate.nodes.iter().any(|n|n.id==id) {return Err("module not found".into());}
                candidate.nodes.retain(|n|n.id!=id);
                candidate.cables.retain(|c|c.from!=id&&c.to!=id);
                if candidate.output.as_deref()==Some(id) {candidate.output=None;}
            }
            "connect" => {candidate.cables.push(Cable{from:string(&args,"from")?.into(),to:string(&args,"to")?.into(),input:string(&args,"input")?.into()});}
            "disconnect" => {
                let to=string(&args,"to")?; let input=string(&args,"input")?;
                let before=candidate.cables.len();
                candidate.cables.retain(|c|!(c.to==to&&c.input==input));
                if before==candidate.cables.len() {return Err("cable not found".into());}
            }
            "set_parameter" => {
                let id=string(&args,"id")?;
                let node=candidate.nodes.iter_mut().find(|n|n.id==id).ok_or("module not found")?;
                let value=args.get("value").and_then(Value::as_f64).ok_or("value must be a number")? as f32;
                node.params.insert(string(&args,"parameter")?.into(),value);
            }
            "set_output" => {candidate.output=Some(string(&args,"id")?.into());}
            _ => return Err(format!("unknown tool: {name}")),
        }
        // Transaction boundary: failed edits never change the patch or revision.
        candidate.compile()?;
        let revision=self.revision.checked_add(1).ok_or("revision exhausted")?;
        self.patch=candidate;
        self.revision=revision;
        Ok(self.snapshot())
    }
}

fn tool(name:&str,description:&str,properties:Value,required:&[&str],read_only:bool)->Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":read_only,"destructiveHint":false,"idempotentHint":read_only,"openWorldHint":false}})
}

pub fn tools()->Value {
    let s=json!({"type":"string"}); let rev=json!({"type":"integer","minimum":0});
    json!([
        tool("catalog","List built-in modules, input ports, parameter defaults and ranges.",json!({}),&[],true),
        tool("inspect_patch","Read the session's patch and revision. Each MCP process owns its own session.",json!({}),&[],true),
        tool("replace_patch","Atomically replace the complete patch. Invalid patches leave state unchanged.",json!({"patch":{"type":"object"},"expected_revision":rev}),&["patch"],false),
        tool("add_module","Add a built-in module by stable ID. Consult catalog for kind and params.",json!({"id":s,"kind":s,"params":{"type":"object","additionalProperties":{"type":"number"}},"expected_revision":rev}),&["id","kind"],false),
        tool("remove_module","Remove a module and its attached cables.",json!({"id":s,"expected_revision":rev}),&["id"],false),
        tool("connect","Connect a source to a named input. Cycles and occupied inputs are rejected.",json!({"from":s,"to":s,"input":s,"expected_revision":rev}),&["from","to","input"],false),
        tool("disconnect","Disconnect a named destination input.",json!({"to":s,"input":s,"expected_revision":rev}),&["to","input"],false),
        tool("set_parameter","Set a finite parameter within catalog limits.",json!({"id":s,"parameter":s,"value":{"type":"number"},"expected_revision":rev}),&["id","parameter","value"],false),
        tool("set_output","Select a module as the mono rendering output.",json!({"id":s,"expected_revision":rev}),&["id"],false),
        tool("save_patch","Save patch JSON inside the workspace. Simple filename only; never overwrites.",json!({"filename":s}),&["filename"],false),
        tool("load_patch","Load workspace patch JSON, validate, then replace the session patch.",json!({"filename":s,"expected_revision":rev}),&["filename"],false),
        tool("render_audio","Render offline from reset to mono 16-bit WAV in workspace. Does not play speakers. Never overwrites.",json!({"filename":s,"seconds":{"type":"number","minimum":0.001,"maximum":30}}),&["filename","seconds"],false),
        tool("benchmark","Measure this patch's offline DSP throughput, excluding compilation. Release builds recommended. No VCV Rack comparison.",json!({"seconds":{"type":"number","minimum":0.01,"maximum":5}}),&[],true)
    ])
}
