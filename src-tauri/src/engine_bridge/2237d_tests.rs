use beebeeb_core::{chunk_stream::{ChunkDecryptor, ChunkEncryptor, ExpectedChunkFormat}, kdf::{FileKey, MasterKey}};
use crate::owner_keys::OwnedKeyRecord;
const ID: &str = "30000000-0000-4000-8000-000000000003";
const ROOT: &str = "10000000-0000-4000-8000-000000000001";
const OBJECT: &str = "90000000-0000-4000-8000-000000000009";
fn random_key() -> FileKey { FileKey::from_bytes([66;32]) }
fn named(key:&FileKey, text:&str, pin:ExpectedChunkFormat)->String {
    beebeeb_core::encrypt::encrypt_name_for(key,text,Some("text/plain"),pin).unwrap()
}
fn frames(key:&FileKey,id:&str,plain:&[u8],pin:ExpectedChunkFormat)->Vec<Vec<u8>> {
    let mut enc=ChunkEncryptor::for_push_with_file_key(FileKey::from_bytes(*key.as_bytes()),id,plain.len() as u64,4).unwrap().with_recorded_pin(pin).unwrap();
    let mut result=Vec::new();
    for bytes in plain.chunks(4) { result.push(enc.push_chunk(bytes).unwrap().data); }
    enc.finish().unwrap();result
}
fn opened(key:&FileKey,id:&str,wire:&[Vec<u8>],pin:ExpectedChunkFormat)->Vec<u8> {
    let mut dec=ChunkDecryptor::for_push_with_file_key(FileKey::from_bytes(*key.as_bytes()),id,pin);
    let mut bytes=Vec::new();for frame in wire {bytes.extend(dec.push_frame(frame).unwrap().data);}
    dec.finish().unwrap();bytes
}
struct ContentState { meta:serde_json::Value, frames:Vec<Vec<u8>>, requests:Vec<RecordedRequest>, uploads:Vec<Vec<u8>>, gone:usize, meta_reads:usize }
struct ContentServer { base:String, state:Arc<Mutex<ContentState>>, stop:Arc<AtomicBool>, thread:Option<thread::JoinHandle<()>> }
impl ContentServer {
    fn start(key:&FileKey,pin:ExpectedChunkFormat)->Self {
        let plain=b"guest bytes!";
        let state=Arc::new(Mutex::new(ContentState {meta:json!({"id":ID,"name_encrypted":named(key,"guest.txt",pin),"size_bytes":plain.len(),"chunk_count":3,"chunk_size_bytes":4,"version_number":1}), frames:frames(key,ID,plain,pin),requests:vec![],uploads:vec![],gone:0,meta_reads:0}));
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
        let base=format!("http://{}",listener.local_addr().unwrap());let stop=Arc::new(AtomicBool::new(false));let signal=stop.clone();let world=state.clone();
        let thread=thread::spawn(move || {while !signal.load(Ordering::Acquire) {match listener.accept() {
            Ok((mut stream,_))=> {stream.set_nonblocking(false).unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let req=read_http_request(&mut stream);let mut w=world.lock().unwrap();
                let (status,body)=if req.method=="GET" && req.path==format!("/api/v1/files/{ID}") {w.meta_reads+=1;(200,serde_json::to_vec(&w.meta).unwrap())}
                else if req.method=="GET" && req.path.contains("/chunks/") {let index=req.path.split("/chunks/").nth(1).unwrap().split('?').next().unwrap().parse::<usize>().unwrap();
                    if w.gone>0 {w.gone-=1;(409,b"{}".to_vec())} else {(200,w.frames[index].clone())}}
                else if req.path.contains("/thumbnail/") {(200,beebeeb_core::encrypt::encrypt_thumbnail_for(&random_key(),b"thumbnail",pin).unwrap())}
                else if req.method=="PUT" && req.path.contains("/uploads/") {w.uploads.push(req.body.clone());(200,b"{}".to_vec())}
                else if req.method=="POST" && req.path.ends_with("/complete") {(200,b"{}".to_vec())}
                else {(404,b"{}".to_vec())};w.requests.push(req);drop(w);
                let header=format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());stream.write_all(header.as_bytes()).unwrap();stream.write_all(&body).unwrap();
            },Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>thread::sleep(Duration::from_millis(2)),Err(e)=>panic!("{e}")
        }}});
        Self {base,state,stop,thread:Some(thread)}
    }
    fn bridge(&self,path:&Path,foreign:bool)->EngineBridge {
        let b=test_bridge_with_api(path,self.base.clone(),[7;32]);
        if foreign {b.db.put_owned_key_record(&OwnedKeyRecord {file_id:ID.into(),name_encrypted:self.state.lock().unwrap().meta["name_encrypted"].as_str().unwrap().into(),owner_wrap:Some(beebeeb_sync::key_resolution::wrap_key_to_owner(&MasterKey::from_bytes([7;32]),ID,&random_key()).unwrap()),foreign_added:true,shared_root_id:Some(ROOT.into()),..Default::default()}).unwrap();}
        seed_bridge_row(&b,ID,"guest.txt",None,FileStatus::CloudOnly,0);b
    }
}
impl Drop for ContentServer {fn drop(&mut self) {self.stop.store(true,Ordering::Release);let result=self.thread.take().unwrap().join();if !thread::panicking(){result.unwrap();}}}
fn runtime()->tokio::runtime::Runtime {tokio::runtime::Runtime::new().unwrap()}
#[test]
fn sf2237d_t5_whole_and_range_open() {
    for foreign in [true,false] {for pin in [ExpectedChunkFormat::Unrecorded,ExpectedChunkFormat::V2] {
        let key=if foreign {random_key()}else {hydration_test_key([7;32],ID)};
        let server=ContentServer::start(&key,pin);let temp=tempfile::tempdir().unwrap();let b=server.bridge(&temp.path().join("db"),foreign);
        let whole=runtime().block_on(b.do_hydrate(ID,None)).unwrap();assert_eq!(&*whole,b"guest bytes!");
        let range=runtime().block_on(b.hydrate_file_range(ID,4,4)).unwrap().unwrap();assert_eq!(&*range.data,b"t by");
    }}
}
#[test]
fn sf2237d_t5_thumbnail_preview_and_versions() {
    let s=ContentServer::start(&random_key(),ExpectedChunkFormat::V2);let temp=tempfile::tempdir().unwrap();let b=s.bridge(&temp.path().join("db"),true);
    assert_eq!(&*runtime().block_on(b.fetch_thumbnail_to_memory(ID,"small")).unwrap(),b"thumbnail");
    let (preview,_)=runtime().block_on(b.remote_content_preview(ID,true));assert_eq!(preview.text.as_deref(),Some("guest bytes!"));
    let meta=s.state.lock().unwrap().meta.clone();assert_eq!(&*runtime().block_on(b.do_hydrate_with_meta(ID,&meta,None)).unwrap(),b"guest bytes!");
}
#[test]
fn sf2237d_t5_rename_and_replacement_keep_key() {
    let s=ContentServer::start(&random_key(),ExpectedChunkFormat::V2);let temp=tempfile::tempdir().unwrap();let b=s.bridge(&temp.path().join("db"),true);
    let name=runtime().block_on(b.encrypt_owned_name(ID,"renamed.txt",ExpectedChunkFormat::V2)).unwrap();
    let m=beebeeb_core::encrypt::decrypt_file_metadata_with_key(&random_key(),&name).unwrap();assert_eq!(m.name,"renamed.txt");assert_eq!(m.chunk_format,ExpectedChunkFormat::V2);
    let key=runtime().block_on(b.file_key_for_download(ID)).unwrap();let wire=frames(&key,ID,b"replacement!",m.chunk_format);
    assert_eq!(opened(&random_key(),ID,&wire,ExpectedChunkFormat::V2),b"replacement!");
}
#[test]
fn sf2237d_t5_move_out_restart_without_folder() {
    let s=ContentServer::start(&random_key(),ExpectedChunkFormat::Unrecorded);let temp=tempfile::tempdir().unwrap();let path=temp.path().join("db");
    drop(s.bridge(&path,true));let b=test_bridge_with_api(&path,s.base.clone(),[7;32]);
    assert_eq!(&*runtime().block_on(b.do_hydrate(ID,None)).unwrap(),b"guest bytes!");
    assert_eq!(s.state.lock().unwrap().requests.iter().filter(|r|r.path.contains("shares")||r.path.contains("shared-folders")).count(),0);
}
#[test]
fn sf2237d_t5_bad_key_never_publishes() {
    let s=ContentServer::start(&random_key(),ExpectedChunkFormat::V2);s.state.lock().unwrap().frames[1][15]^=1;
    let temp=tempfile::tempdir().unwrap();let b=s.bridge(&temp.path().join("db"),true);let final_path=temp.path().join("result.txt");
    assert!(runtime().block_on(b.hydrate_file(ID,&final_path,&[temp.path()])).is_err());assert!(!final_path.exists());
    assert_eq!(b.db.get_file_contract_state(ID).unwrap().unwrap().cache_bytes,0);
}
#[test]
fn sf2237d_t5_request_and_sharedwithme_unchanged() {
    let key=hydration_test_key([7;32],ID);let s=ContentServer::start(&key,ExpectedChunkFormat::Unrecorded);let temp=tempfile::tempdir().unwrap();let b=s.bridge(&temp.path().join("db"),false);
    assert_eq!(&*runtime().block_on(b.do_hydrate(ID,None)).unwrap(),b"guest bytes!");
    let sender=MasterKey::from_bytes([8;32]);let recipient=MasterKey::from_bytes([7;32]);let sp=beebeeb_core::opaque::derive_x25519_public(&beebeeb_core::opaque::derive_x25519_private(&sender));
    let rp=beebeeb_core::opaque::derive_x25519_public(&beebeeb_core::opaque::derive_x25519_private(&recipient));
    let secret=beebeeb_core::opaque::x25519_shared_secret(&beebeeb_core::opaque::derive_x25519_private(&sender),&rp).unwrap();let share=beebeeb_core::opaque::derive_share_key(&secret,ID.as_bytes());
    let wrapped=beebeeb_core::encrypt::encrypt_chunk_raw(&FileKey::from_bytes(*share),random_key().as_bytes()).unwrap();
    let unwrapped=unwrap_direct_shared_file_key(&[7;32],&base64::engine::general_purpose::STANDARD.encode(sp),ID,&base64::engine::general_purpose::STANDARD.encode(wrapped)).unwrap();assert_eq!(unwrapped.as_bytes(),random_key().as_bytes());
}
#[test]
fn sf2237d_t5_pin_and_object_change() {
    let key=hydration_test_key([7;32],ID);let s=ContentServer::start(&key,ExpectedChunkFormat::V2);let temp=tempfile::tempdir().unwrap();let b=s.bridge(&temp.path().join("db"),false);
    s.state.lock().unwrap().frames=frames(&key,ID,b"guest bytes!",ExpectedChunkFormat::Unrecorded);
    assert!(runtime().block_on(b.do_hydrate(ID,None)).is_err(),"v2 metadata cannot accept v1 frames");
    // The restart helper is the shared 1643 entrypoint for whole/range/preview.
    let tries=std::sync::atomic::AtomicUsize::new(0);
    let result: anyhow::Result<()> = runtime().block_on(b.with_one_restart(ID,json!({}),|_| {tries.fetch_add(1,Ordering::SeqCst);async {Err(anyhow::Error::new(crate::download_pin::VersionGone))}}));
    assert!(result.is_err());let object_restarts=tries.load(Ordering::SeqCst)-1;assert!(object_restarts<=1);
}
#[test]
fn sf2237d_1943_truncated_v2_never_publishes() {
    let key=hydration_test_key([7;32],ID);let s=ContentServer::start(&key,ExpectedChunkFormat::V2);{
        let mut w=s.state.lock().unwrap();w.frames.pop();w.meta["chunk_count"]=json!(2);w.meta["size_bytes"]=json!(8);
    }
    let temp=tempfile::tempdir().unwrap();let b=s.bridge(&temp.path().join("db"),false);let final_path=temp.path().join("result.txt");
    assert!(runtime().block_on(b.hydrate_file(ID,&final_path,&[temp.path()])).is_err());assert!(!final_path.exists());assert_eq!(b.db.get_file_contract_state(ID).unwrap().unwrap().cache_bytes,0);
}
#[test]
fn sf2237d_l7b_custody_never_uses_legacy_reader() {
    use base64::Engine as _;
    LEGACY_CHILD_READS.with(|n| n.set(0));
    let folder=FileKey::from_bytes([23;32]);
    let wrap=beebeeb_sync::key_resolution::wrap_child_key(&folder,ROOT,ID,1,&random_key()).unwrap();
    let encoded=base64::engine::general_purpose::STANDARD.encode(wrap);
    let got=unwrap_folder_child_for_format(folder.as_bytes(),ROOT,ID,1,&encoded,true);
    assert_eq!(LEGACY_CHILD_READS.with(|n| n.get()),0,"custody must never try the unbound reader");
    assert_eq!(got.unwrap().as_bytes(),random_key().as_bytes());
    let legacy=base64::engine::general_purpose::STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder,random_key().as_bytes()).unwrap());
    assert!(unwrap_folder_child_for_format(folder.as_bytes(),ROOT,ID,1,&legacy,true).is_err());
    assert_eq!(LEGACY_CHILD_READS.with(|n| n.get()),0);
    assert_eq!(unwrap_folder_child_for_format(folder.as_bytes(),ROOT,ID,1,&legacy,false).unwrap().as_bytes(),random_key().as_bytes());
    assert_eq!(LEGACY_CHILD_READS.with(|n| n.get()),1);
}
