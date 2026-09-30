// Real mounted main entry with deterministic IPC; run against baseline and candidate.
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href)
const [bundlePath, evidence] = process.argv.slice(2)
const bundle = await fs.readFile(bundlePath, 'utf8')
const caps = JSON.parse(await fs.readFile(new URL('./fixtures/desktop-capabilities.json', import.meta.url)))['windows-nsis']
const browser = await chromium.launch({executablePath: process.env.CHROME_PATH, headless:true})
try {
 const page = await browser.newPage()
 await page.route('http://fixture/**', r => r.fulfill({body:'<html><body><div id="root"></div></body></html>',contentType:'text/html'}))
 await page.goto('http://fixture/?window=main-app&platform=windows')
 await page.evaluate(caps => {
  window.fixture = {account:'A',revision:1,calls:[]}
  window.__TAURI_INTERNALS__ = {transformCallback:()=>1,unregisterCallback:()=>{},invoke:async name=>{
   const f=window.fixture; f.calls.push([f.account,name]);
   if(name==='plugin:event|listen')return 1
   if(name==='desktop_capabilities')return caps
   if(name==='desktop_config')return {theme:'light'}
   if(name==='sync_status')return {logged_in:f.account!==null,session_revision:f.revision,engine:'running',syncing:0,cloud_only:0,conflicts:0}
   if(name==='account_usage')return {used_bytes:f.account==='A'?55000000:32,quota_bytes:5000000000,percentage:0.01}
   if(name==='account_region')return {region:{city:f.account==='A'?'Helsinki':'Ede',country:'Netherlands'}}
   if(name==='known_folder_onboarding_seen')return true
   if(name==='get_known_folder_backup')return []
   throw new Error('fixture unavailable '+name)
  }}
 },caps)
 await page.addScriptTag({content:bundle})
 await page.getByText('55 MB / 5 GB',{exact:true}).first().waitFor()
 // Switch between status polls: never rely on observing logged_in=false.
 await page.evaluate(()=>{window.fixture.account='B';window.fixture.revision=3})
 await page.getByText('32 B / 5 GB',{exact:true}).first().waitFor({timeout:12000})
 assert.equal(await page.getByText('55 MB / 5 GB',{exact:true}).count(),0)
 assert.ok(await page.evaluate(()=>window.fixture.calls.filter(([a,c])=>a==='B'&&c==='account_region').length)>0)
 await fs.writeFile(evidence,JSON.stringify({passed:1,failed:0,text:await page.locator('body').innerText(),calls:await page.evaluate(()=>window.fixture.calls)},null,2))
 console.log('account switch browser: 1 passed; 0 failed')
}finally{await browser.close()}
