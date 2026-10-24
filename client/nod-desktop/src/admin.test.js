// @vitest-environment node
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { afterEach, describe, expect, it, vi } from "vitest";
const html = readFileSync("../../server/nod-server/assets/admin.html", "utf8").replace('"__NOD_EMOJI_DATA__"', readFileSync("../../server/nod-server/assets/admin-emoji-data.json", "utf8").trim());
const pages = [];
afterEach(() => {
  pages.splice(0).forEach((page) => page.window.close());
});
function page(overrides = {}, { privateHttp = false, setup = () => {} } = {}) {
  const fixtures = {
    "/summary": {},
    "/users": {
      users: [
        { id: "owner", name: "Owner", subscribed_channel_ids: ["default"] },
      ],
    },
    "/channels": {
      channels: [{ id: "default", name: "Default", emoji: "🔔" }],
    },
    "/devices": { devices: [] },
    "/issuer-tokens": { tokens: [] },
    "/settings": {},
    "/activity": {
      requests: [
        {
          id: "r1",
          title: '<img src=x onerror="alert(1)">',
          channel_id: "default",
          status: "pending",
        },
      ],
      deliveries: [],
      audit: { healthy: true },
    },
    "/users/owner/enrollment-codes": {
      code: "ABCDEFGH",
      expires_at: "2030-01-01T00:00:00Z",
    },
  };
  const fetch = vi.fn(async (url, init) => {
    const path = String(url).replace("/api/v1/admin", "");
    const handler = overrides[path];
    const data = handler ? await handler(init) : fixtures[path];
    return {
      ok: !data?.__status || data.__status < 400,
      status: data?.__status || 200,
      text: async () => JSON.stringify(data ?? {}),
    };
  });
  const dom = new JSDOM(html, {
    url: privateHttp
      ? "http://192.168.1.10:8767/admin"
      : "https://nod.test/admin",
    runScripts: "dangerously",
    beforeParse(window) {
      window.fetch = fetch;
      window.TextEncoder = TextEncoder;
      setup(window);
      if (privateHttp)
        Object.defineProperty(window.crypto, "randomUUID", {
          value: undefined,
        });
    },
  });
  pages.push(dom);
  return { dom, fetch, get: (id) => dom.window.document.getElementById(id) };
}
async function ready(get) {
  await vi.waitFor(() => expect(get("status").textContent).toBe("Ready"));
}
describe("embedded admin UI", () => {
  it("omits None and authors exactly one valid typed default without changing display fields", async () => {
    const {dom,get} = page(); await ready(get);
    const payload = () => dom.window.buildTestRequestPayload();
    const change = () => get('testAnswerType').dispatchEvent(new dom.window.Event('change',{bubbles:true}));
    get('testTitle').value = 'Batch'; get('testOptionPreset').value = 'approval';
    expect(payload()).not.toHaveProperty('answer_field');
    get('testAnswerType').value = 'choice'; change();
    expect(JSON.parse(JSON.stringify(payload().answer_field))).toEqual({kind:'choice',label:'Batch',choices:[{id:'small',label:'Small'},{id:'large',label:'Large'}],default:{kind:'choice',choice_id:'small'}});
    expect(payload().fields).toEqual([]);
    get('testAnswerChoiceDefault').value = ''; expect(payload).toThrow('Choose a default');
    get('testAnswerChoices').value = 'small: Small\nsmall: Large'; change(); expect(payload).toThrow('unique');
    get('testAnswerChoices').value = 'small: Same\nlarge: Same'; change(); expect(payload).toThrow('unique');
    get('testAnswerType').value = 'number'; change(); get('testAnswerLabel').value='Workers';
    for (const value of ['4','1.5','-0.25']) { get('testAnswerDefault').value=value; expect(payload().answer_field.default).toEqual({kind:'number',decimal:value}); }
    for (const value of ['', '1.50','-0','-0.0','01','1e2','+1',' 1','NaN','Infinity','1234567890123','0.0000001']) { get('testAnswerDefault').value=value; expect(payload).toThrow('exact decimal'); expect(dom.window.document.activeElement.id).toBe('testAnswerDefault'); }
    get('testAnswerType').value = 'date'; change();
    get('testAnswerDefault').value = '2026-10-25'; expect(payload().answer_field.default).toEqual({kind:'date',date:'2026-10-25'});
    get('testAnswerDefault').value = '2026-02-29'; expect(payload).toThrow('real date');
    get('testAnswerType').value = 'date_time'; change();
    get('testAnswerDefault').value = '2026-10-25T09:30:00Z'; expect(payload().answer_field.default).toEqual({kind:'date_time',instant:'2026-10-25T09:30:00Z'});
    for (const value of ['2026-10-25T09:30:01Z','2026-10-25T09:30:00+00:00','2026-02-30T09:30:00Z']) { get('testAnswerDefault').value=value; expect(payload).toThrow('UTC minute'); }
    get('testAnswerDefault').value = '2026-10-25T09:30:00Z'; get('testOptionPreset').value = 'approval_text'; expect(payload).toThrow('ordinary outcome');
  });
  it("keeps typed drafts across navigation and quiet reads and changes attempt identity with the default", async () => {
    const bodies=[];
    const {dom,get} = page({'/test-requests':async init=>{bodies.push(JSON.parse(init.body));throw Error('lost');}}); await ready(get);
    get('testTitle').value='Batch';get('testOptionPreset').value='approval';get('testAnswerType').value='choice';get('testAnswerType').dispatchEvent(new dom.window.Event('change',{bubbles:true}));
    const submit=()=>get('testRequestForm').dispatchEvent(new dom.window.Event('submit',{cancelable:true}));
    submit(); await vi.waitFor(()=>expect(bodies).toHaveLength(1));
    await dom.window.loadAll();
    expect(get('testAnswerType').value).toBe('choice');
    get('testAnswerChoiceDefault').value='large'; submit();await vi.waitFor(()=>expect(bodies).toHaveLength(2));
    expect(bodies[1].answer_field.default.choice_id).toBe('large');expect(bodies[1].idempotency_key).not.toBe(bodies[0].idempotency_key);
  });
  it("escapes tagged receipt values alongside notes with shared receipt fallback", async () => {
    const {dom,get}=page({'/activity':async()=>({requests:[{id:'typed',title:'Batch',channel_id:'default',status:'answered',decisions:[],decision:{option_label:'Apply',answer_value:{kind:'choice',choice_id:'<img src=x>'},text:'<script>notes</script>'}}],deliveries:[]})});await ready(get);
    dom.window.document.querySelector('[data-inspect-request="typed"]').click();
    const inspector=dom.window.document.querySelector('[data-inspector="activity"]');
    expect(inspector.textContent).toContain('Answer (choice): <img src=x>');expect(inspector.textContent).toContain('<script>notes</script>');expect(inspector.querySelector('img,script')).toBeNull();
  });
  it("runs with local assets and renders activity content as text", async () => {
    const { dom, get } = page();
    await ready(get);
    expect(dom.window.document.querySelector("script[src]")).toBeNull();
    expect(get("activityRequests").querySelector("img")).toBeNull();
    expect(get("activityRequests").textContent).toContain("<img src=x");
    expect(get("auditHealth").textContent).toBe("Audit logging healthy");
  });
  it("creates an explicit one-use enrollment link", async () => {
    const { dom, get } = page();
    await ready(get);
    get("deviceEnrollmentUser").value = "owner";
    get("deviceEnrollmentForm").dispatchEvent(
      new dom.window.Event("submit", { cancelable: true }),
    );
    await vi.waitFor(() =>
      expect(get("deviceEnrollmentSecret").querySelector("a")).not.toBeNull(),
    );
    const link = new URL(get("deviceEnrollmentSecret").querySelector("a").href);
    expect(link.protocol).toBe("nod:");
    expect(link.searchParams.get("server")).toBe("https://nod.test");
    expect(link.searchParams.get("code")).toBe("ABCDEFGH");
  });
  it("reuses the attempt identity after an uncertain create without allowing parallel submission", async () => {
    const bodies = [];
    const { dom, get } = page({
      "/test-requests": async (init) => {
        bodies.push(JSON.parse(init.body));
        throw new Error("connection lost");
      },
    });
    await ready(get);
    get("testTitle").value = "Retry test";
    get("testExpires").value = "3600";
    const submit = () =>
      get("testRequestForm").dispatchEvent(
        new dom.window.Event("submit", { cancelable: true }),
      );
    submit();
    submit();
    await vi.waitFor(() =>
      expect(get("testRequestSubmit").disabled).toBe(false),
    );
    submit();
    await vi.waitFor(() => expect(bodies).toHaveLength(2));
    expect(bodies[0].idempotency_key).toBeTruthy();
    expect(bodies[0].idempotency_key).toBe(bodies[1].idempotency_key);
    expect(bodies[0].expires_at).toBe(bodies[1].expires_at);
  });
  it("creates test requests from private HTTP admin hosts without randomUUID", async () => {
    const bodies = [];
    const { dom, get } = page(
      {
        "/test-requests": async (init) => {
          bodies.push(JSON.parse(init.body));
          return { request_id: "lan-test" };
        },
      },
      { privateHttp: true },
    );
    await ready(get);
    get("testTitle").value = "Private LAN test";
    get("testRequestForm").dispatchEvent(
      new dom.window.Event("submit", { cancelable: true }),
    );
    await vi.waitFor(() => expect(bodies).toHaveLength(1));
    expect(bodies[0].idempotency_key).toMatch(/^admin-test-[0-9a-f]{32}$/);
    await vi.waitFor(() =>
      expect(get("testRequestSubmit").disabled).toBe(false),
    );
    expect(get("status").textContent).toContain("Test request created");
  });
  it("offers selected enrollment-link text when private HTTP cannot copy automatically", async () => {
    const { dom, get } = page({}, { privateHttp: true });
    await ready(get);
    get("deviceEnrollmentUser").value = "owner";
    get("deviceEnrollmentForm").dispatchEvent(
      new dom.window.Event("submit", { cancelable: true }),
    );
    await vi.waitFor(() =>
      expect(get("status").textContent).toBe("Enrollment code created"),
    );
    const button = Array.from(
      get("deviceEnrollmentSecret").querySelectorAll("button"),
    ).find((item) => item.textContent === "Copy enrollment link");
    button.click();
    const field = get("deviceEnrollmentSecret").querySelector("textarea");
    expect(field.value).toContain("nod://enroll?");
    expect(field.value).toContain("code=ABCDEFGH");
    expect(field.selectionEnd).toBe(field.value.length);
    expect(get("status").textContent).toContain("Press Ctrl+C or Command+C");
  });
});

describe('A Inset admin safety and tasks', () => {
  const change = (dom, node) => node.dispatchEvent(new dom.window.Event('change', {bubbles:true}));
  it.each([
    ['shared receipt with empty per-user list', {decisions:[],decision:{actor_user_id:'owner',option_label:'Shared approval',text:'Shared reason'}}, ['Shared approval','Shared reason']],
    ['per-user receipts', {decisions:[{decision:{actor_user_id:'owner',option_label:'Owner approved',text:'Owner reason'}},{decision:{actor_user_id:'other',option_label:'Other rejected',text:'Other reason'}}],decision:{option_label:'Do not duplicate shared fallback'}}, ['Owner approved','Owner reason','Other rejected','Other reason']]
  ])('receipt inspector displays %s', async (_name, receipts, expected) => {
    const {dom,get}=page({'/activity':()=>({requests:[{id:'receipt-card',title:'Receipt card',channel_id:'default',status:'answered',...receipts}],deliveries:[],audit:{healthy:true}})});
    await ready(get);
    dom.window.document.querySelector('[data-inspect-request="receipt-card"]').click();
    const detail=dom.window.document.querySelector('[data-inspector="activity"]');
    expected.forEach(value=>expect(detail.textContent).toContain(value));
    expect(detail.textContent).not.toContain('No decision receipts');
    expect(detail.textContent).not.toContain('Do not duplicate shared fallback');
  });
  it('focuses subscriptions and rejects a late response for a previously closed user dialog', async () => {
    let release; const pending = new Promise(resolve => release = resolve);
    const {dom,get}=page({
      '/users':()=>({users:[{id:'owner',name:'Owner'},{id:'other',name:'Other'}]}),
      '/users/owner/subscriptions':()=>pending,
      '/users/other/subscriptions':()=>({channels:[{id:'default',name:'Default',subscribed:false}]})
    });
    await ready(get);
    dom.window.document.querySelector('[data-manage-subscriptions="owner"]').click();
    expect(dom.window.document.activeElement).toBe(get('subscriptionSearch'));
    get('closeSubscriptionModal').click();
    dom.window.document.querySelector('[data-manage-subscriptions="other"]').click();
    await vi.waitFor(()=>expect(get('subscriptionList').querySelector('input')).not.toBeNull());
    release({channels:[{id:'default',name:'Wrong old user data',subscribed:true}]});
    await new Promise(resolve=>setTimeout(resolve,20));
    expect(get('subscriptionModalTitle').textContent).toContain('Other');
    expect(get('subscriptionList').textContent).not.toContain('Wrong old user');
    expect(get('subscriptionList').querySelector('input').checked).toBe(false);
    dom.window.document.dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
    expect(get('subscriptionModalBackdrop').classList.contains('hidden')).toBe(true);
  });
  it('separates active access from searchable known revocation history', async () => {
    const {dom,get} = page({'/issuer-tokens': () => ({tokens:[
      {id:'a',name:'Build agent',scopes:['requests:write:default']},
      {id:'r',name:'Old deploy',scopes:['requests:read:default'],revoked_at:'2026-10-01T00:00:00Z'}
    ]})});
    await ready(get);
    expect(get('tokensBody').textContent).toContain('Build agent');
    expect(get('tokensBody').textContent).not.toContain('Old deploy');
    dom.window.document.querySelector('[data-token-mode="revoked"]').click();
    const search = dom.window.document.querySelector('[aria-label="Search revoked tokens"]');
    search.value = 'Old deploy'; search.dispatchEvent(new dom.window.Event('input'));
    expect([...get('tokensBody').rows].filter(row=>!row.hidden)).toHaveLength(1);
    expect(get('tokensBody').textContent).not.toContain('Build agent');
    dom.window.document.querySelector('[data-inspect-token="r"]').click();
    const detail = dom.window.document.querySelector('[data-inspector="token"]');
    expect(detail.textContent).toContain('requests:read:default');
    expect(detail.querySelector('[data-revoke-token]')).toBeNull();
    search.value = 'unknown'; search.dispatchEvent(new dom.window.Event('input'));
    expect([...get('tokensBody').rows].filter(row=>!row.hidden)).toHaveLength(0);
  });
  it('bundles full keyword/category/skin search and roundtrips complex pasted Unicode', async () => {
    const saved = [];
    const {dom,get} = page({'/channels': init => {
      if (init?.body) saved.push(JSON.parse(init.body));
      return {channels:[{id:'default',name:'Default',emoji:saved.at(-1)?.emoji || '🔔'}]};
    }});
    await ready(get);
    dom.window.document.querySelector('[data-edit-channel="default"]').click();
    get('emojiPickerToggle').click();
    expect(get('emojiCount').textContent).toMatch(/of [3-9]\d{3} emoji/);
    get('emojiSearch').value='astronaut medium skin'; get('emojiSearch').dispatchEvent(new dom.window.Event('input'));
    expect(get('emojiResults').textContent).toContain('👩🏽‍🚀');
    get('emojiCategory').value='9'; change(dom,get('emojiCategory'));
    expect(get('emojiResults').children).toHaveLength(0);
    get('emojiCategory').value=''; get('emojiSearch').value=''; change(dom,get('emojiCategory'));
    dom.window.document.dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
    expect(get('emojiPickerPopover').classList.contains('hidden')).toBe(true);
    expect(get('channelModalBackdrop').classList.contains('hidden')).toBe(false);
    get('emojiPickerToggle').click();
    get('channelEmojiPicker').value='123456789'; get('applyChannelEmoji').click();
    expect(get('emojiValidation').textContent).toContain('at most 8');
    expect(get('channelEmoji').value).toBe('🔔');
    dom.window.document.dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
    for (const emoji of ['👩🏽‍❤️‍💋‍👨🏻','🇺🇸','👨‍👩‍👧‍👦','👍🏿']) {
      get('emojiPickerToggle').click(); get('channelEmojiPicker').value=emoji; get('applyChannelEmoji').click();
      expect(get('channelEmoji').value).toBe(emoji);
      expect(get('channelEmojiPreview').textContent).toBe(emoji);
      get('channelForm').dispatchEvent(new dom.window.Event('submit',{cancelable:true}));
      await vi.waitFor(()=>expect(saved.at(-1)?.emoji).toBe(emoji));
      await vi.waitFor(()=>expect(get('channelModalBackdrop').classList.contains('hidden')).toBe(true));
      dom.window.document.querySelector('[data-edit-channel="default"]').click();
      expect(get('channelEmojiPicker').value).toBe(emoji);
    }
  });
  it('selects only enabled subscribers while retaining explicit override capability', async () => {
    const {dom,get} = page({'/users':()=>({users:[{id:'owner',name:'Owner',subscribed_channel_ids:['default']},{id:'other',name:'Other',subscribed_channel_ids:[]}]})});
    await ready(get);
    get('testRecipientsSpecific').checked=true; change(dom,get('testRecipientsSpecific'));
    get('selectTestRecipients').click();
    const inputs = [...get('testRecipientsList').querySelectorAll('input')];
    expect(inputs.map(node=>node.checked)).toEqual([true,false]);
    inputs[1].checked=true; change(dom,inputs[1]);
    expect(get('testRecipientSummary').textContent).toContain('2 selected');
  });
  it('clears secrets/dialogs/drafts immediately and rejects late responses after logout', async () => {
    let resolve;
    const pending = new Promise(r=>resolve=r);
    const {dom,get} = page({'/issuer-tokens': init=>init?.method==='POST' ? pending : {tokens:[]}});
    await ready(get);
    get('openTokenModal').click(); get('tokenName').value='Secret issuer';
    get('tokenForm').dispatchEvent(new dom.window.Event('submit',{cancelable:true}));
    await vi.waitFor(()=>expect(get('status').textContent).not.toBe('Checking session'));
    get('logoutButton').click(); resolve({token:'SHOULD-NOT-REAPPEAR'});
    await new Promise(r=>setTimeout(r,30));
    expect(get('adminView').classList.contains('hidden')).toBe(true);
    expect(get('tokenSecret').textContent).toBe('');
    expect(get('tokenName').value).toBe('');
    expect(dom.window.document.querySelector('.modal-backdrop:not(.hidden)')).toBeNull();
    expect(dom.window.document.body.textContent).not.toContain('SHOULD-NOT-REAPPEAR');
  });
  it('clears protected context on expiry while retaining known state on transient read failure', async () => {
    let status=200;
    const {get} = page({'/activity':()=>status===200 ? {requests:[{id:'known',title:'Known request'}],deliveries:[]} : {__status:status,message:'Read failed'}});
    await ready(get); status=500; get('refreshButton').click();
    await vi.waitFor(()=>expect(get('status').textContent).toBe('Read failed'));
    expect(get('activityRequests').textContent).toContain('Known request');
    status=401; get('refreshButton').click();
    await vi.waitFor(()=>expect(get('adminView').classList.contains('hidden')).toBe(true));
    expect(get('activityRequests').textContent).toBe('');
  });
  it('reconciles per-user partial membership saves and resumes failed intent without retrying success', async () => {
    let fail=true;
    const members={owner:false,other:false}; const writes=[];
    const {dom,get}=page({
      '/users':()=>({users:Object.keys(members).map(id=>({id,name:id,subscribed_channel_ids:members[id]?['default']:[]}))}),
      '/users/owner/subscriptions':init=>{writes.push('owner');members.owner=true;return{};},
      '/users/other/subscriptions':init=>{writes.push('other');if(fail)throw Error('Offline');members.other=true;return{};}
    });
    await ready(get);
    dom.window.document.querySelector('[data-manage-channel-members="default"]').click();
    get('addAllChannelMembers').click(); get('saveChannelMembershipChanges').click();
    await vi.waitFor(()=>expect(get('saveChannelMembershipChanges').textContent).toBe('Retry failed'));
    expect(get('channelMembershipList').textContent).toContain('Saved');
    expect(get('channelMembershipList').textContent).toContain('Failed');
    get('closeChannelMembershipModal').click();
    dom.window.document.querySelector('[data-manage-channel-members="default"]').click();
    expect(get('channelMembershipList').querySelector('[data-channel-member-user="other"]').checked).toBe(true);
    fail=false; get('saveChannelMembershipChanges').click();
    await vi.waitFor(()=>expect(get('status').textContent).toBe('Channel members saved'));
    expect(writes).toEqual(['owner','other','other']);
  });
  it('does not restore a session from a late login response or leave the password in the DOM', async () => {
    let resolve; const pending=new Promise(r=>resolve=r);
    const {dom,get}=page({'/admin/session':init=>init?.method==='POST'?pending:{}});
    await ready(get); dom.window.eval('showLogin()');
    get('adminToken').value='ONE-TIME-PASSWORD';
    get('loginForm').dispatchEvent(new dom.window.Event('submit',{cancelable:true}));
    get('logoutButton').click(); resolve({ok:true});
    await new Promise(r=>setTimeout(r,20));
    expect(get('adminToken').value).toBe('');
    expect(get('adminView').classList.contains('hidden')).toBe(true);
    expect(dom.window.document.body.innerHTML).not.toContain('ONE-TIME-PASSWORD');
  });
  it('closing a membership save stops unstarted writes and keeps the remaining draft', async () => {
    let release; const pending=new Promise(r=>release=r); const writes=[]; let owner=false;
    const {dom,get}=page({
      '/users':()=>({users:[{id:'owner',name:'Owner',subscribed_channel_ids:owner?['default']:[]},{id:'other',name:'Other',subscribed_channel_ids:[]}]}),
      '/users/owner/subscriptions':async()=>{writes.push('owner');await pending;owner=true;return{};},
      '/users/other/subscriptions':()=>{writes.push('other');return{};}
    });
    await ready(get);
    dom.window.document.querySelector('[data-manage-channel-members="default"]').click();
    get('addAllChannelMembers').click();get('saveChannelMembershipChanges').click();
    await vi.waitFor(()=>expect(writes).toEqual(['owner']));
    get('cancelChannelMembershipChanges').click();release({});
    await vi.waitFor(()=>expect(dom.window.eval('state.membershipDrafts.default.saving')).toBe(false));
    expect(writes).toEqual(['owner']);
    dom.window.document.querySelector('[data-manage-channel-members="default"]').click();
    expect(get('channelMembershipList').textContent).toContain('Not attempted');
    expect(get('channelMembershipList').querySelector('[data-channel-member-user="other"]').checked).toBe(true);
  });
  it('rebases clean and dirty membership drafts onto the current roster', async () => {
    const {dom,get}=page(); await ready(get);
    const open=()=>dom.window.openChannelMembershipModal('default');
    const checked=id=>get('channelMembershipList').querySelector(`[data-channel-member-user="${id}"]`)?.checked;
    open(); get('cancelChannelMembershipChanges').click();
    dom.window.eval('state.users = [{id:"new",name:"New",subscribed_channel_ids:["default"]},{id:"owner",name:"Owner",subscribed_channel_ids:[]}]');
    open(); expect(checked('new')).toBe(true); expect(checked('owner')).toBe(false);
    get('addAllChannelMembers').click(); get('cancelChannelMembershipChanges').click();
    dom.window.eval('state.users = state.users.filter(user => user.id !== "new")');
    open(); expect(checked('new')).toBeUndefined(); expect(checked('owner')).toBe(true);
    expect(dom.window.eval('state.channelMembershipModal.original.owner')).toBe(false);
  });
  it('refreshes fully saved drafts and reconciles an uncertain commit without a second PUT', async () => {
    let actual=false, unavailable=false; const writes=[];
    const {dom,get}=page({
      '/users':()=>{if(unavailable)throw Error('Offline');return {users:[{id:'owner',name:'Owner',subscribed_channel_ids:actual?['default']:[]}]};},
      '/users/owner/subscriptions':()=>{writes.push('owner');actual=true;unavailable=true;throw Error('Uncertain');}
    });
    await ready(get); dom.window.openChannelMembershipModal('default');
    get('addAllChannelMembers').click();get('saveChannelMembershipChanges').click();
    await vi.waitFor(()=>expect(get('saveChannelMembershipChanges').textContent).toBe('Retry failed'));
    get('cancelChannelMembershipChanges').click();dom.window.openChannelMembershipModal('default');
    expect(get('channelMembershipList').querySelector('input').checked).toBe(true);
    expect(get('channelMembershipList').textContent).toContain('Failed');
    unavailable=false;get('saveChannelMembershipChanges').click();
    await vi.waitFor(()=>expect(get('status').textContent).toBe('Channel members saved'));
    expect(writes).toEqual(['owner']);
    expect(dom.window.eval('state.users[0].subscribed_channel_ids.includes("default")')).toBe(true);
    expect(dom.window.eval('state.users.filter(user => userSubscribedToChannel(user,"default")).length')).toBe(1);
    get('cancelChannelMembershipChanges').click();
    dom.window.eval('state.users = [{id:"fresh",name:"Fresh",subscribed_channel_ids:[]}]');
    dom.window.openChannelMembershipModal('default');
    expect(get('channelMembershipList').textContent).toContain('Fresh');
    expect(get('channelMembershipList').querySelector('[data-channel-member-user="owner"]')).toBeNull();
  });
  it('a deferred channel save cannot change another channel modal controls or status', async () => {
    let release; const pending=new Promise(r=>release=r); let actual=false;
    const {dom,get}=page({
      '/users':()=>({users:[{id:'owner',name:'Owner',subscribed_channel_ids:actual?['default']:[]}]}),
      '/users/owner/subscriptions':async()=>{await pending;actual=true;return{};}
    });
    await ready(get); dom.window.openChannelMembershipModal('default');
    get('addAllChannelMembers').click();get('saveChannelMembershipChanges').click();
    const saving=dom.window.eval('state.channelMembershipModal');
    get('cancelChannelMembershipChanges').click();dom.window.openChannelMembershipModal('other','Other');
    get('addAllChannelMembers').click();
    const status=get('status').textContent, label=get('saveChannelMembershipChanges').textContent;
    get('channelMembershipSearch').focus(); release();
    await vi.waitFor(()=>expect(saving.saving).toBe(false));
    expect(get('status').textContent).toBe(status);
    expect(get('saveChannelMembershipChanges').textContent).toBe(label);
    expect(get('saveChannelMembershipChanges').disabled).toBe(false);
    expect(dom.window.document.activeElement).toBe(get('channelMembershipSearch'));
    expect(dom.window.eval('state.channelMembershipModal.channelId')).toBe('other');
  });
  it('a pending clipboard failure cannot recreate copy fallback DOM after logout', async () => {
    let reject; const pending=new Promise((_,r)=>reject=r);
    const {dom,get}=page({}, {setup:window=>Object.defineProperty(window.navigator,'clipboard',{value:{writeText:()=>pending}})});
    await ready(get);
    dom.window.eval('showSecret("tokenSecret","Access token","COPY-SECRET")');
    get('tokenSecret').querySelector('button').click();
    get('logoutButton').click(); reject(Error('Denied'));
    await new Promise(r=>setTimeout(r,20));
    expect(dom.window.document.querySelector('[data-copy-fallback]')).toBeNull();
    expect(get('tokenSecret').textContent).toBe('');
    expect(dom.window.document.body.innerHTML).not.toContain('COPY-SECRET');
  });
  it('quiet GET reconciliation preserves drafts, recipient intent and focused controls', async () => {
    const {dom,get,fetch}=page(); await ready(get);
    get('testTitle').value='Draft title';
    get('testBody').value='Draft body';
    get('testRecipientsSpecific').checked=true; change(dom,get('testRecipientsSpecific'));
    get('selectTestRecipients').click();
    get('testTitle').focus();
    const control=get('testTitle'); const before=fetch.mock.calls.length;
    await dom.window.eval('loadAll({quiet:true})');
    expect(get('testTitle')).toBe(control);
    expect(dom.window.document.activeElement).toBe(control);
    expect(control.value).toBe('Draft title');
    expect(get('testBody').value).toBe('Draft body');
    expect(get('testRecipientsList').querySelector('input').checked).toBe(true);
    expect(fetch.mock.calls.slice(before).every(([,init])=>!init.method || init.method==='GET')).toBe(true);
  });
});
