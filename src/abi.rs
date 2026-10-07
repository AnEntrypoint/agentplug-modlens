use std::alloc::{alloc, dealloc, Layout};
use std::mem;

use serde_json::{json, Value};

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_fs_read(path_ptr: *const u8, path_len: u32) -> u64;
    fn host_fs_read_base64(path_ptr: *const u8, path_len: u32) -> u64;
    fn host_fetch(url_ptr: *const u8, url_len: u32, opts_ptr: *const u8, opts_len: u32) -> u64;
    fn host_env_get(key_ptr: *const u8, key_len: u32) -> u64;
    fn host_now_ms() -> u64;
}

#[no_mangle]
pub extern "C" fn plugkit_alloc(len: u32) -> u32 {
    if len == 0 {
        return 0;
    }
    let layout = Layout::from_size_align(len as usize, mem::align_of::<u8>()).unwrap();
    unsafe { alloc(layout) as u32 }
}

#[no_mangle]
pub extern "C" fn plugkit_free(ptr: u32, len: u32) {
    if ptr == 0 || len == 0 {
        return;
    }
    let layout = Layout::from_size_align(len as usize, mem::align_of::<u8>()).unwrap();
    unsafe { dealloc(ptr as *mut u8, layout) };
}

fn read_str(ptr: u32, len: u32) -> String {
    if len == 0 {
        return String::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(slice).into_owned()
}

fn unpack(packed: u64) -> Option<String> {
    let p = (packed & 0xffff_ffff) as u32;
    let l = (packed >> 32) as u32;
    if p == 0 || l == 0 {
        return None;
    }
    let bytes = unsafe { Vec::from_raw_parts(p as *mut u8, l as usize, l as usize) };
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn return_json(v: Value) -> u64 {
    let bytes = v.to_string().into_bytes();
    let len = bytes.len();
    let ptr = plugkit_alloc(len as u32);
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, len) };
    (ptr as u64 & 0xffff_ffff) | ((len as u64) << 32)
}

pub fn read_text(path: &str) -> Option<String> {
    unpack(unsafe { host_fs_read(path.as_ptr(), path.len() as u32) })
}

pub fn read_base64(path: &str) -> Option<String> {
    unpack(unsafe { host_fs_read_base64(path.as_ptr(), path.len() as u32) })
}

pub fn fetch(url: &str, opts: &Value) -> Value {
    let opts = opts.to_string();
    let packed = unsafe { host_fetch(url.as_ptr(), url.len() as u32, opts.as_ptr(), opts.len() as u32) };
    unpack(packed)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({"ok": false, "status": 0, "error": "host_fetch returned nothing"}))
}

pub fn env(key: &str) -> Option<String> {
    unpack(unsafe { host_env_get(key.as_ptr(), key.len() as u32) })
}

pub fn now_ms() -> u64 {
    unsafe { host_now_ms() }
}

#[no_mangle]
pub extern "C" fn plugin_call(verb_ptr: u32, verb_len: u32, body_ptr: u32, body_len: u32) -> u64 {
    let verb = read_str(verb_ptr, verb_len);
    let body: Value = serde_json::from_str(&read_str(body_ptr, body_len)).unwrap_or_else(|_| json!({}));
    match verb.as_str() {
        "read_image" | "read" | "analyze" | "vision" => return_json(crate::analyze::read_image(&body)),
        "doctor" => return_json(crate::analyze::doctor(&body)),
        "capabilities" => return_json(crate::analyze::capabilities()),
        _ => return_json(json!({"ok": false, "error": "unknown_verb", "verb": verb})),
    }
}
