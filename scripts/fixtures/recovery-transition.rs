#![allow(dead_code,unused_imports)]
use std::{sync::{Arc,Mutex,atomic::{AtomicBool,AtomicUsize,Ordering}},future::{Future,poll_fn},task::{Context,Poll,Wake,Waker},pin::Pin};
struct Guard; impl Drop for Guard {fn drop(&mut self){LOCKED.store(false,Ordering::SeqCst);}}
static LOCKED:AtomicBool=AtomicBool::new(false);
struct Transition;impl Transition {async fn lock(&self)->Guard {poll_fn(|_|if !LOCKED.swap(true,Ordering::SeqCst){Poll::Ready(Guard)}else{Poll::Pending}).await}}
static SESSION_TRANSITION:Transition=Transition;
struct Auth; struct Attempt;
static SERIAL:Mutex<()>=Mutex::new(());
static SUPPRESS_CANCELLATION:AtomicBool=AtomicBool::new(false);
static CANCELLED:AtomicBool=AtomicBool::new(false);
impl Auth {fn begin(&self)->Result<Attempt,String>{Ok(Attempt)}fn validate(&self,_:&Attempt)->Result<(),String>{if CANCELLED.load(Ordering::SeqCst){Err("cancelled".into())}else{Ok(())}}fn close(&self){CANCELLED.store(true,Ordering::SeqCst);}}
impl Attempt {async fn run<T>(&self,work:impl Future<Output=Result<T,String>>)->Result<T,String>{let mut work=Box::pin(work);poll_fn(|cx|if !SUPPRESS_CANCELLATION.load(Ordering::SeqCst) && CANCELLED.load(Ordering::SeqCst){Poll::Ready(Err("cancelled".into()))}else{work.as_mut().poll(cx)}).await}}
static AUTH_ATTEMPTS:Auth=Auth;
struct Commands;impl Commands {fn ensure_reactivation_allowed(&self)->Result<(),String>{Ok(())}}
static SESSION_COMMANDS:Commands=Commands;
mod windows_cf {pub fn ensure_reactivation_allowed()->Result<(),String>{Ok(())}}
mod tauri {pub struct AppHandle;}
macro_rules! ignore_log {($($t:tt)*)=>{}}
mod tracing {pub(crate) use ignore_log as info;pub(crate) use ignore_log as warn;}
struct Session{token:String,master_key:[u8;32],email:Option<String>}
struct Account{id:String,session:Mutex<Option<Session>>,auth_email:Mutex<Option<String>>}
struct AppState{account:Account}
impl AppState{fn active_account(&self)->Result<&Account,String>{Ok(&self.account)}}
type State<'a,T>=&'a T;
static PERSISTED:AtomicUsize=AtomicUsize::new(0);static RUNNERS:AtomicUsize=AtomicUsize::new(0);
static VERIFY_ENTERED:AtomicBool=AtomicBool::new(false);static REPLY:AtomicBool=AtomicBool::new(false);
fn persist_vault_key_to_keychain(_:&str,_:[u8;32])->Result<(),String>{PERSISTED.fetch_add(1,Ordering::SeqCst);Ok(())}
fn load_session_token_from_keychain(_:&str)->Result<Option<String>,String>{Ok(Some("synthetic".into()))}
fn set_auth_present(_:&AppState,_:bool){}
fn platform_keychain_store_for(_:&str){}
struct AuthVault;impl AuthVault{fn new(_:())->Self{Self}fn store_account_email(&self,_:&str)->Result<(),String>{Ok(())}}
mod runner {pub fn api_base_url()->String{"fixture".into()}}
mod api_client {pub fn provenance_headers(){}}
mod reqwest {pub struct Client;impl Client{pub fn builder()->Self{Self}pub fn timeout(self,_:std::time::Duration)->Self{self}pub fn default_headers(self,_:())->Self{self}pub fn build(self)->Result<Self,String>{Ok(self)}}}
mod zeroize {pub struct Zeroizing<T>(T);impl<T> Zeroizing<T>{pub fn new(v:T)->Self{Self(v)}}impl<T> std::ops::Deref for Zeroizing<T>{type Target=T;fn deref(&self)->&T{&self.0}}}
async fn verify_vault_key_from_phrase(_: &reqwest::Client,_:&str,_:&str,_:&str)->Result<zeroize::Zeroizing<[u8;32]>,String>{VERIFY_ENTERED.store(true,Ordering::SeqCst);poll_fn(|_|if REPLY.load(Ordering::SeqCst){Poll::Ready(Ok(zeroize::Zeroizing::new([7;32])))}else{Poll::Pending}).await}
async fn provision_vault_key_from_phrase<P:FnOnce([u8;32])->Result<(),String>>(c:&reqwest::Client,u:&str,t:&str,p:&str,persist:P)->Result<[u8;32],String>{let key=verify_vault_key_from_phrase(c,u,t,p).await?;persist(*key)?;Ok(*key)}
async fn start_engine_if_possible(_:tauri::AppHandle,_:&State<'_,AppState>,_:String,_:[u8;32],_:&Guard)->Result<(),String>{RUNNERS.fetch_add(1,Ordering::SeqCst);Ok(())}
struct Noop;impl Wake for Noop{fn wake(self:Arc<Self>){}}
#[test]fn round7_held_recovery_reply_allows_prompt_teardown_without_key_or_runner(){
 let _serial=SERIAL.lock().unwrap_or_else(|e| e.into_inner()); reset();
 let state=AppState{account:Account{id:"fixture".into(),session:Mutex::new(None),auth_email:Mutex::new(None)}};
 let w=Waker::from(Arc::new(Noop));let mut cx=Context::from_waker(&w);
 let mut unlock=Box::pin(desktop_unlock_with_recovery_phrase(tauri::AppHandle,&state,"synthetic".into()));
 assert!(unlock.as_mut().poll(&mut cx).is_pending());assert!(VERIFY_ENTERED.load(Ordering::SeqCst));
 let mut teardown=Box::pin(async{let _transition=SESSION_TRANSITION.lock().await;AUTH_ATTEMPTS.close();});
 let began=std::time::Instant::now();
 assert!(teardown.as_mut().poll(&mut cx).is_ready(),"lock/sign-out cannot acquire SESSION_TRANSITION while recovery verifier is held");
 assert!(began.elapsed()<std::time::Duration::from_secs(1));
 assert!(matches!(unlock.as_mut().poll(&mut cx),Poll::Ready(Err(_))));
 REPLY.store(true,Ordering::SeqCst);
 assert_eq!((PERSISTED.load(Ordering::SeqCst),RUNNERS.load(Ordering::SeqCst)),(0,0));
}

fn reset(){LOCKED.store(false,Ordering::SeqCst);CANCELLED.store(false,Ordering::SeqCst);VERIFY_ENTERED.store(false,Ordering::SeqCst);REPLY.store(false,Ordering::SeqCst);SUPPRESS_CANCELLATION.store(false,Ordering::SeqCst);PERSISTED.store(0,Ordering::SeqCst);RUNNERS.store(0,Ordering::SeqCst);}
#[test]fn round7_late_verified_recovery_key_is_fenced_before_persistence(){
 let _serial=SERIAL.lock().unwrap_or_else(|e| e.into_inner());reset();
 let state=AppState{account:Account{id:"fixture".into(),session:Mutex::new(None),auth_email:Mutex::new(None)}};
 let w=Waker::from(Arc::new(Noop));let mut cx=Context::from_waker(&w);
 let mut unlock=Box::pin(desktop_unlock_with_recovery_phrase(tauri::AppHandle,&state,"synthetic".into()));
 assert!(unlock.as_mut().poll(&mut cx).is_pending());assert!(VERIFY_ENTERED.load(Ordering::SeqCst));
 let mut acquire=Box::pin(SESSION_TRANSITION.lock());
 let guard=match acquire.as_mut().poll(&mut cx){Poll::Ready(g)=>g,_=>panic!("verifier holds transition")};
 REPLY.store(true,Ordering::SeqCst);
 assert!(unlock.as_mut().poll(&mut cx).is_pending()); // verified reply queued behind teardown
 AUTH_ATTEMPTS.close();drop(guard);
 SUPPRESS_CANCELLATION.store(true,Ordering::SeqCst); // independently test the final generation fence
 assert!(matches!(unlock.as_mut().poll(&mut cx),Poll::Ready(Err(_))),"late verification bypassed generation fence");
 assert_eq!((PERSISTED.load(Ordering::SeqCst),RUNNERS.load(Ordering::SeqCst)),(0,0));
}
