// Real, offscreen Chrome + actual embedded admin DOM, mocked API only.
// node src/admin.browser.mjs /absolute/private/evidence-directory
import {spawn} from 'node:child_process';
import {readFileSync,writeFileSync,mkdirSync,mkdtempSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
const out=process.argv[2]; if(!out?.startsWith('/')) throw Error('Explicit absolute evidence directory required');
mkdirSync(out,{recursive:true});
const profile=mkdtempSync('/private/var/folders/pz/lrml3ths3mg3mf4bb75rmcvc0000gn/T/opencode/nod-admin-chrome-');
const assets=new URL('../../../server/nod-server/assets/',import.meta.url);
const notices=['admin-emoji-NOTICE.txt','admin-emoji-LICENSE.txt','admin-emoji-UNICODE-LICENSE.txt'].map(name=>readFileSync(new URL(name,assets),'utf8')).join('\n\n');
const html=readFileSync(new URL('admin.html',assets),'utf8').replace('"__NOD_EMOJI_DATA__"',readFileSync(new URL('admin-emoji-data.json',assets),'utf8').trim()).replace('__NOD_EMOJI_NOTICES__',notices.replaceAll('&','&amp;').replaceAll('<','&lt;'));
const scriptHash=createHash('sha256').update(html.split('<script>')[1].split('</script>')[0]).digest('base64');
const documentHtml=html.replace('<head>',`<head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'sha256-${scriptHash}'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'self'; object-src 'none'">`);
const users=Array.from({length:12},(_,i)=>({id:i?'user'+i:'owner',name:i?'Operator '+i:'Owner',device_count:1,subscribed_channel_ids:['default'],created_at:'2026-10-01T12:00:00Z'}));
const channels=Array.from({length:12},(_,i)=>({id:i?'channel'+i:'default',name:['Default','Builds','Steam','Heater','Operations','Security','Lab','Tests','Reviews','Backups','Network','Delivery'][i],emoji:['🔔','🛠️','🎮','🔥'][i%4]}));
const fixtures={
 '/summary':{},'/users':{users},'/channels':{channels},
 '/devices':{devices:users.map((u,i)=>({id:'device'+i,user_id:u.id,user_name:u.name,name:'Mac '+i,platform:'macOS',notification_sound:'default',last_seen_at:'2026-10-23T12:00:00Z'}))},
 '/issuer-tokens':{tokens:Array.from({length:12},(_,i)=>({id:'token'+i,name:i<8?'Build agent '+i:'Old deploy '+i,scopes:['requests:write:default','requests:read:default'],created_at:'2026-10-01T12:00:00Z',revoked_at:i<8?null:'2026-10-20T12:00:00Z'}))},
 '/settings':{notification_delivery_mode:'push',retention_days:30},
 '/activity':{requests:Array.from({length:9},(_,i)=>({id:'request'+i,title:'Deploy approval '+i,channel_id:'default',status:i?'answered':'pending',created_at:'2026-10-23T12:00:00Z',updated_at:'2026-10-23T12:05:00Z',decisions:i?[{decision:{actor_user_id:'owner',option_label:'Approve',resolved_at:'2026-10-23T12:05:00Z',text:'Ready'}}]:[]})),deliveries:[{request_id:'request0',device_id:'device0',status:'failed',attempts:2,error:'Provider unavailable'}],audit:{healthy:true},integrations:{publisher_status:'connected',lifecycle_total:20,lifecycle_pending:1,lifecycle_retrying:0,lifecycle_max_attempts:1,lifecycle_oldest_pending_at:'2026-10-23T12:00:00Z',callbacks_queued:0,callbacks_retrying:1,callbacks_blocked_policy:0,callbacks_sending:0,callbacks_delivered:9,callbacks_max_attempts:2,callbacks_oldest_pending_at:'2026-10-23T12:00:00Z'}}
};
if (process.argv.includes('--value-only')) {
 fixtures['/activity'].requests = [{id:'value-receipt',title:'Batch receipt (mock)',channel_id:'default',status:'answered',decisions:[],decision:{actor_user_id:'owner',option_label:'Apply',answer_value:{kind:'choice',choice_id:'large'},text:'Keep <canary> & notes',resolved_at:'2026-10-24T00:00:00Z'}}];
}
if (process.argv.includes('--receipt-only')) {
 fixtures['/activity'].requests = [
  {id:'shared-receipt',title:'Shared receipt card',channel_id:'default',status:'answered',decisions:[],decision:{actor_user_id:'owner',option_label:'Shared approval',text:'Shared reason',resolved_at:'2026-10-24T00:00:00Z'}},
  {id:'per-user-receipts',title:'Per-user receipt card',channel_id:'default',status:'answered',decisions:[{decision:{actor_user_id:'owner',option_label:'Owner approved',text:'Owner reason'}},{decision:{actor_user_id:'other',option_label:'Other rejected',text:'Other reason'}}],decision:{option_label:'Do not duplicate shared fallback'}}
 ];
}
const chrome=spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',['--headless=new','--remote-debugging-pipe',`--user-data-dir=${profile}`,'--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--disable-extensions','about:blank'],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let seq=0,buffer='',pending=new Map();chrome.stderr.resume();
chrome.stdio[4].on('data',chunk=>{buffer+=chunk;let i;while((i=buffer.indexOf('\0'))>=0){const raw=buffer.slice(0,i);buffer=buffer.slice(i+1);if(!raw)continue;const msg=JSON.parse(raw);if(msg.id&&pending.has(msg.id)){const p=pending.get(msg.id);pending.delete(msg.id);msg.error?p.reject(Error(JSON.stringify(msg.error))):p.resolve(msg.result)}}});
const call=(method,params={},sessionId)=>new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve,reject});chrome.stdio[3].write(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})})+'\0')});
const deadline=setTimeout(()=>chrome.kill(),60000),reports=[];
try {
 const {targetId}=await call('Target.createTarget',{url:'about:blank'}),{sessionId}=await call('Target.attachToTarget',{targetId,flatten:true});
 const cmd=(m,p)=>call(m,p,sessionId);
 const ev=async expression=>{const r=await cmd('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value};
 await cmd('Page.enable');await cmd('Runtime.enable');
 await ev(`window.fetch=async (url,init)=>{const path=String(url).split('/api/v1/admin')[1];const data=(${JSON.stringify(fixtures)})[path]||{};return {ok:true,status:200,text:async()=>JSON.stringify(data)}}`);
 await cmd('Page.setDocumentContent',{frameId:(await cmd('Page.getFrameTree')).frameTree.frame.id,html:documentHtml});
 await ev(`new Promise(resolve=>{function check(){if(document.querySelector('#status')?.textContent==='Ready')requestAnimationFrame(()=>requestAnimationFrame(resolve));else requestAnimationFrame(check)}check()})`);
 const settle=()=>ev('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
 const key=async(key,code=key,virtual=0)=>{await cmd('Input.dispatchKeyEvent',{type:'keyDown',key,code,windowsVirtualKeyCode:virtual,...(key==='Enter'?{text:'\r',unmodifiedText:'\r'}:{})});await cmd('Input.dispatchKeyEvent',{type:'keyUp',key,code,windowsVirtualKeyCode:virtual});await settle()};
 const assert=async(expr,name)=>{if(!await ev(expr))throw Error(name)};
 const click=async selector=>{const point=await ev(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.scrollIntoView({block:'nearest'});const r=e.getBoundingClientRect();if(!r.width||e.disabled)throw Error('Unavailable target');const hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);if(hit!==e&&!e.contains(hit))throw Error('Target obscured: '+${JSON.stringify(selector)});return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);await cmd('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',clickCount:1,...point});await cmd('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...point});await settle()};
  if (process.argv.includes('--value-only')) {
   await cmd('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:false});
   await click('[data-tab="test-request"]');
   await ev(`document.querySelector('#testAnswerType').value='choice';document.querySelector('#testAnswerType').dispatchEvent(new Event('change',{bubbles:true}));document.querySelector('#testAnswerType').focus()`);
   await key('Tab','Tab',9);
   await assert(`document.activeElement.id==='testAnswerLabel'`,'Keyboard reaches answer label from type');
   await ev(`document.querySelector('#testTitle').value='Which batch?';document.querySelector('#testOptionPreset').value='approval'`);
   await assert(`JSON.stringify(buildTestRequestPayload().answer_field)===JSON.stringify({kind:'choice',label:'Batch',choices:[{id:'small',label:'Small'},{id:'large',label:'Large'}],default:{kind:'choice',choice_id:'small'}})`,'Exact choice payload with visible default');
   await assert(`document.querySelector('#testAnswerPreview').textContent.includes('Small')`,'Visible default preview');
   await ev(`document.querySelector('#testAnswerChoices').value='small: Small';document.querySelector('#testAnswerChoices').dispatchEvent(new Event('input',{bubbles:true}));try{buildTestRequestPayload()}catch(e){window.invalidMessage=e.message}`);
   await assert(`document.activeElement.id==='testAnswerChoices'&&window.invalidMessage.includes('2–12')`,'Invalid choices focus repair');
   await ev(`document.querySelector('#testAnswerChoices').value='small: Small\\nlarge: Large';document.querySelector('#testAnswerChoices').dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('#testAnswerType').value='number';document.querySelector('#testAnswerType').dispatchEvent(new Event('change',{bubbles:true}));document.querySelector('#testAnswerLabel').value='Workers';document.querySelector('#testAnswerDefault').value='1.50';try{buildTestRequestPayload()}catch(e){window.invalidMessage=e.message}`);
   await assert(`document.activeElement.id==='testAnswerDefault'&&window.invalidMessage.includes('exact decimal')`,'Decimal rejection without rounding');
   await ev(`document.querySelector('#testAnswerDefault').value='1.5';document.querySelector('#testAnswerDefault').dispatchEvent(new Event('input',{bubbles:true}))`);
   await assert(`buildTestRequestPayload().answer_field.default.decimal==='1.5'&&document.documentElement.scrollWidth<=innerWidth`,'Canonical number and narrow layout');
   {const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(out+'/390-value-authoring.png',Buffer.from(image.data,'base64'));}
   await click('[data-tab="activity"]'); await click('[data-inspect-request="value-receipt"]');
   await assert(`(()=>{const e=document.querySelector('[data-inspector="activity"]');return e.textContent.includes('Answer (choice): large')&&e.textContent.includes('Keep <canary> & notes')&&!e.querySelector('canary')&&document.documentElement.scrollWidth<=innerWidth})()`,'Typed receipt and escaped notes at narrow width');
   {const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(out+'/390-value-receipt.png',Buffer.from(image.data,'base64'));}
   const checks={proof:'Actual headless Chrome, mocked API only — NOT live backend/signature proof; native select popup key selection not established, type set through DOM then real Tab traversal',checks:['keyboard type-to-label traversal','exact small default payload','visible preview','invalid choice focus','canonical decimal rejection and repair','390px authoring layout','typed shared receipt with escaped notes'],sourceHash:createHash('sha256').update(html).digest('hex')};
   writeFileSync(out+'/value-browser-checks.json',JSON.stringify(checks,null,2));console.log(JSON.stringify(checks,null,2));
  } else if (process.argv.includes('--receipt-only')) {
  await cmd('Emulation.setDeviceMetricsOverride',{width:1440,height:900,deviceScaleFactor:1,mobile:false});
  await click('[data-tab="activity"]');await click('[data-inspect-request="shared-receipt"]');
  await assert(`(()=>{const text=document.querySelector('[data-inspector="activity"]').textContent;return text.includes('Shared approval')&&text.includes('Shared reason')&&!text.includes('No decision receipts')})()`,'Shared receipt survives empty decisions array');
  const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(`${out}/1440-shared-receipt-inspector.png`,Buffer.from(image.data,'base64'));
  await click('[data-close-integrations]');await click('[data-inspect-request="per-user-receipts"]');
  await assert(`(()=>{const text=document.querySelector('[data-inspector="activity"]').textContent;return ['Owner approved','Owner reason','Other rejected','Other reason'].every(value=>text.includes(value))&&!text.includes('Do not duplicate shared fallback')})()`,'Per-user receipts take precedence without duplication');
  const checks={checks:['shared decision with empty decisions array visible','per-user list visible without shared fallback duplication'],sourceHash:createHash('sha256').update(html).digest('hex'),screenshot:`${out}/1440-shared-receipt-inspector.png`};
  writeFileSync(`${out}/receipt-inspector-checks.json`,JSON.stringify(checks,null,2));console.log(JSON.stringify(checks,null,2));
 } else {
 for(const [width,height] of [[1440,900],[1280,800],[390,844]]) {
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
  for(const view of ['activity','channels','tokens','users-devices','test-request','settings']) {
   await click(`[data-tab="${view}"]`);
   const fit=await ev(`({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight,small:[...document.querySelectorAll('button,input:not([type=checkbox]):not([type=radio]),select')].filter(e=>{const r=e.getBoundingClientRect();return r.width&&r.height&&(r.width<43.5||r.height<43.5)}).map(e=>({id:e.id,text:e.textContent,w:e.getBoundingClientRect().width,h:e.getBoundingClientRect().height}))})`);
   reports.push({width,view,...fit});if(!process.argv.includes('--geometry-only')&&(fit.scrollWidth>width||fit.small.length))throw Error('Geometry '+JSON.stringify(reports.at(-1)));
   { const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(`${out}/${width}-${view}.png`,Buffer.from(image.data,'base64')); }
  }
 }
 if (process.argv.includes('--geometry-only')) { console.log(JSON.stringify(reports,null,2)); process.exitCode = reports.some(row=>row.scrollWidth>row.width||row.small.length) ? 1 : 0; }
 else {
 for (const [width,height] of [[1440,900],[1280,800]]) {
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
  const boundary=async(name)=>{
   await settle();
   const geometry=await ev(`({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight,viewportHeight:innerHeight})`);
   if(geometry.scrollWidth>width||geometry.height>height)throw Error('Boundary geometry '+name+JSON.stringify(geometry));
   reports.push({boundary:name,...geometry});
   const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(`${out}/${width}-${name}.png`,Buffer.from(image.data,'base64'));
  };
  await click('[data-tab="tokens"]');await click('[data-token-mode="active"]');await click('[data-inspect-token="token0"]');await boundary('token-inspector');
  await click('[data-close-token]');await click('[data-token-mode="revoked"]');await click('[data-inspect-token="token8"]');await boundary('revoked-inspector');
  await click('[data-close-token]');await click('[data-tab="activity"]');await click('[data-inspect-integrations]');await boundary('callbacks-inspector');await click('[data-close-integrations]');
  await click('[data-activity-mode="push"]');await click('[data-inspect-push="0"]');await boundary('push-inspector');await click('[data-close-integrations]');
  await click('[data-activity-mode="requests"]');await click('[data-inspect-request="request1"]');await boundary('receipts-inspector');await click('[data-close-integrations]');
  await click('[data-tab="users-devices"]');await click('[data-user-mode="devices"]');await boundary('devices');await click('[data-user-mode="users"]');
  await click('[data-tab="test-request"]');await click('[data-test-mode="recipients"]');await boundary('test-recipients');
  await click('[data-test-mode="delivery"]');await boundary('test-delivery');await click('[data-test-mode="message"]');
 }
 for (const [width,height] of [[1440,900],[1280,800],[390,844]]) {
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
  await click('[data-tab="channels"]');await click('[data-edit-channel="default"]');await click('#emojiPickerToggle');
  await assert(`document.querySelector('#channelSubmit').disabled`,'Unapplied emoji edit interlocks parent Save');
  await assert(`(()=>{const r=document.querySelector('.channel-modal').getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight})()`,'Channel dialog stays in viewport');
  await assert(`getComputedStyle(document.querySelector('#channelEmojiPicker')).height==='44px'&&document.querySelector('#emojiResults').scrollHeight>document.querySelector('#emojiResults').clientHeight`,'Bounded full emoji results with reachable paste input');
  const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(`${out}/${width}-emoji-picker.png`,Buffer.from(image.data,'base64'));
  await key('Escape','Escape',27);await key('Escape','Escape',27);
 }
 await cmd('Emulation.setDeviceMetricsOverride',{width:1440,height:900,deviceScaleFactor:1,mobile:false});
 await click('[data-tab="tokens"]');await click('[data-token-mode="active"]');await click('[data-inspect-token="token0"]');
 await assert(`document.activeElement.textContent==='Build agent 0'&&document.querySelector('[data-inspector="token"]').textContent.includes('Revoke Build agent 0')`,'Token inspector/focus');
 await click('[data-token-mode="revoked"]');
 await assert(`!document.querySelector('#tokensBody').textContent.includes('Build agent')`,'Revoked excludes active');
 await click('[data-tab="channels"]');await click('[data-edit-channel="default"]');await click('#emojiPickerToggle');
 await cmd('Input.insertText',{text:'astronaut'});await settle();await key('Tab','Tab',9);await key('Tab','Tab',9);
 await ev(`document.querySelector('#emojiResults button').focus()`);await key('ArrowRight','ArrowRight',39);
 await assert(`document.activeElement===document.querySelectorAll('#emojiResults button')[1]`,'Emoji arrow keyboard');
 await key('Enter','Enter',13);await assert(`!state.emojiPickerOpen&&document.querySelector('#channelEmoji').value!=='🔔'`,'Keyboard emoji selection');
 await click('#emojiPickerToggle');await key('Escape','Escape',27);await assert(`!state.emojiPickerOpen&&!document.querySelector('#channelModalBackdrop').classList.contains('hidden')`,'Picker Escape only');
 await ev(`document.querySelector('#closeChannelModal').focus()`);
 await cmd('Input.dispatchKeyEvent',{type:'keyDown',key:'Tab',code:'Tab',windowsVirtualKeyCode:9,modifiers:8});await cmd('Input.dispatchKeyEvent',{type:'keyUp',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});
 await assert(`document.activeElement.id==='channelSubmit'`,'Modal reverse Tab containment');
 await key('Tab','Tab',9);await assert(`document.activeElement.id==='closeChannelModal'&&document.activeElement.matches(':focus-visible')&&getComputedStyle(document.activeElement).outlineWidth==='3px'`,'Modal Tab containment and focus outline');
 await key('Escape','Escape',27);await assert(`document.querySelector('#channelModalBackdrop').classList.contains('hidden')`,'Second Escape closes parent');
 await click('[data-tab="activity"]');await click('[data-inspect-integrations]');
 const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(`${out}/1440-activity-inspector.png`,Buffer.from(image.data,'base64'));
 await click('[data-tab="users-devices"]');await click('[data-user-mode="users"]');await click('[data-manage-subscriptions="owner"]');
 await assert(`document.activeElement.id==='subscriptionSearch'&&!document.querySelector('#subscriptionModalBackdrop').classList.contains('hidden')`,'Subscriptions opens with keyboard focus');
 await key('Escape','Escape',27);await assert(`document.querySelector('#subscriptionModalBackdrop').classList.contains('hidden')&&document.activeElement.dataset.manageSubscriptions==='owner'`,'Subscriptions Escape restores opener');
 // Adversarial stateful API remains entirely mocked: no real server or mutations.
 await ev(`window.__originalFetch=window.fetch;window.__users=${JSON.stringify(users)};window.__writes=[];window.__fail=true;window.__hold=false;
 window.fetch=async(url,init)=>{
  const path=String(url).split('/api/v1/admin')[1];
  if(path==='/users')return {ok:true,status:200,text:async()=>JSON.stringify({users:window.__users})};
  const match=path?.match(/^\\/users\\/([^/]+)\\/subscriptions$/);
  if(match&&init?.method==='PUT'){
   const id=match[1];window.__writes.push(id);
   if(window.__hold)await new Promise(resolve=>{window.__release=resolve});
   if(window.__fail&&id==='user1')throw Error('Mock offline');
   const updates=JSON.parse(init.body).updates;const user=window.__users.find(user=>user.id===id);
   for(const update of updates){user.subscribed_channel_ids=user.subscribed_channel_ids.filter(value=>value!==update.channel_id);if(update.subscribed)user.subscribed_channel_ids.push(update.channel_id)}
   return {ok:true,status:200,text:async()=>'{}'};
  }
  return window.__originalFetch(url,init);
 }`);
 await click('[data-tab="channels"]');await click('[data-manage-channel-members="channel1"]');
 await click('[data-channel-member-user="owner"]');await click('[data-channel-member-user="user1"]');await click('#saveChannelMembershipChanges');
 await assert(`document.querySelector('#channelMembershipList').textContent.includes('Saved')&&document.querySelector('#channelMembershipList').textContent.includes('Failed')&&document.querySelector('#saveChannelMembershipChanges').textContent==='Retry failed'`,'Partial save shows individual outcomes');
 {const image=await cmd('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});writeFileSync(`${out}/1440-membership-partial.png`,Buffer.from(image.data,'base64'));}
 await click('#closeChannelMembershipModal');await click('[data-manage-channel-members="channel1"]');
 await assert(`document.querySelector('[data-channel-member-user="user1"]').checked`,'Failed intent resumes');
 await ev('window.__fail=false');await click('#saveChannelMembershipChanges');
 await assert(`JSON.stringify(window.__writes)==='["owner","user1","user1"]'&&document.querySelector('#status').textContent==='Channel members saved'`,'Retry reconciles and does not repeat saved user');
 await click('#closeChannelMembershipModal');await click('[data-manage-channel-members="channel2"]');
 await click('[data-channel-member-user="owner"]');await click('[data-channel-member-user="user1"]');await ev('window.__writes=[];window.__hold=true');await click('#saveChannelMembershipChanges');
 await click('#cancelChannelMembershipChanges');await ev('window.__hold=false;window.__release()');await settle();
 await assert(`JSON.stringify(window.__writes)==='["owner"]'&&document.querySelector('#status').textContent.includes('1 member changes remain')`,'Close stops unstarted writes without false success');
 await click('[data-manage-channel-members="channel2"]');await assert(`document.querySelector('#channelMembershipList').textContent.includes('Not attempted')&&document.querySelector('[data-channel-member-user="user1"]').checked`,'Unattempted draft resumes');await click('#closeChannelMembershipModal');
 // Late clipboard rejection must not recreate detached secret DOM after logout.
 await ev(`showSecret('tokenSecret','Access token','FICTIONAL-COPY-SECRET');window.__copyButton=document.querySelector('#tokenSecret button');Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:()=>new Promise((resolve,reject)=>{window.__rejectCopy=reject})}})`);
 await click('[data-tab="tokens"]');await click('#tokenSecret button');
 await click('[data-tab="users-devices"]');await click('[data-user-mode="devices"]');await assert(`document.querySelector('#openDeviceEnrollmentModal').getBoundingClientRect().width>0`,'Device task mode');
 await click('#logoutButton');await assert(`document.querySelector('#adminView').classList.contains('hidden')&&!document.querySelector('.modal-backdrop:not(.hidden)')`,'Logout clears protected DOM');
 await ev(`window.__rejectCopy(Error('Mock clipboard failure'))`);await settle();
 await assert(`!document.querySelector('[data-copy-fallback]')&&!document.body.innerHTML.includes('FICTIONAL-COPY-SECRET')&&!window.__copyButton.hasAttribute('data-copy-secret')`,'Late clipboard failure clears DOM and detached copy attribute');
 writeFileSync(`${out}/browser-checks.json`,JSON.stringify({reports,checks:['actual Chrome hit-tested mouse navigation','1440/1280 all primary views','390 fallback no document overflow','44px form/action targets','token inspector keyboard focus','revoked isolation','emoji keyboard arrows/Enter','nested Escape order','emoji Save interlock','modal Tab containment and visible focus','subscriptions focus/Escape/opener restoration','partial member save outcomes/reconciliation/retry/resume','close stops unstarted member writes','pending clipboard logout fencing','device mode','logout'],sourceHash:createHash('sha256').update(html).digest('hex')},null,2));
 console.log(JSON.stringify(reports,null,2));
 }
 }
} finally {clearTimeout(deadline);await call('Browser.close').catch(()=>{});chrome.kill();}
