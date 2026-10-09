import { expect, test, type Page, type Locator } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// #4499: real Chromium and checkout assets, deterministic protocol projections.
// This suite proves the queue UI contract, not a real agent launch.
const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
// A browser-check URL may point at the Hub; these fixtures exercise Project assets.
const liveUrl = configuredUrl
  ? new URL(new URL(configuredUrl).pathname === "/" ? new URL(APP_URL).pathname : new URL(configuredUrl).pathname, configuredUrl).toString()
  : undefined;
const errors = new WeakMap<Page, string[]>();
test.use({ viewport: { width: 1600, height: 1100 } });
test.beforeEach(async ({ page }, info) => {
  const captured: string[] = [];
  errors.set(page, captured);
  page.on("pageerror", error => captured.push(error.message));
  page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme),
    info.project.name.includes("light") ? "light" : "dark");
  if (!liveUrl) await installEmbeddedRoutes(page);
  await installBackend(page, {
    includeAgent: info.title.startsWith("T-7a:"),
    observeSkipped: info.title.startsWith("four columns preserve"),
  });
  await page.goto(liveUrl || APP_URL);
  await expect(page.locator(".issue-queue-board")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
});
test.afterEach(async ({ page }, info) => {
  const screenshotPath = info.outputPath(`queue-board-${info.project.name}.png`);
  await page.screenshot({path:screenshotPath});
  await info.attach(`queue-board-${info.project.name}`, {path:screenshotPath, contentType:"image/png"});
  expect(errors.get(page), "zero console/page errors").toEqual([]);
});
const column = (page: Page, phase: string) => page.locator(`[data-queue-column="${phase}"]`);
const row = (page: Page, number: number) => page.locator(`.knowledge-row[data-issue-number="${number}"]`);
const messages = (page: Page) => page.evaluate(() => (window as any).__queueMessages.filter((message: any) =>
  ["issue_monitor_queue_push", "issue_monitor_queue_remove", "issue_monitor_queue_move", "set_issue_monitor_auto_refill"].includes(message.kind)));
async function confirm(page: Page, numbers: number[], extra = {}) {
  await page.evaluate(({numbers, extra}) => (window as any).__queueConfirm(numbers, extra), {numbers, extra});
}
async function drag(page: Page, number: number, target: Locator) {
  await row(page, number).dragTo(target);
}

test("four columns preserve labelled controls, provenance, empty guidance and narrow scrolling", async ({ page }) => {
  await expect(page.locator("[data-queue-column]")).toHaveCount(4);
  await expect(column(page,"backlog")).toContainText("First backlog issue");
  await expect(row(page,1)).toContainText("Skipped");
  await expect(row(page,1)).toContainText("not selected in this terminal queue");
  await expect(row(page,1)).toBeVisible();
  await page.screenshot({path:test.info().outputPath("queue-observations.png")});
  await expect(column(page,"queued")).toContainText("operator");
  await expect(column(page,"queued")).toContainText("auto-refill");
  await expect(column(page,"active")).toContainText("Running issue");
  await expect(column(page,"done")).toContainText("Completed issue");
  await expect(page.locator('[data-issue-filter], [data-issue-lane-filter]')).toHaveCount(0);
  await expect(page.getByRole("button", {name:"Kanban",exact:true})).toBeVisible();
  await expect(page.getByRole("button", {name:"Split",exact:true})).toBeVisible();
  const searchBox=await page.locator(".knowledge-search").boundingBox();
  const modeBox=await page.locator(".knowledge-view-mode").boundingBox();
  expect(Math.abs(searchBox!.y-modeBox!.y), "search and view mode share one filter row").toBeLessThan(2);
  await expect(page.locator('[data-action="monitor-settings"]')).toHaveText("⚙ Settings");
  await expect(page.locator('[data-action="refresh-knowledge"]')).toHaveText("↻ Refresh");
  await expect(page.getByRole("group",{name:"Monitor status",exact:true})).toBeVisible();
  await expect(page.getByRole("group",{name:"Monitor controls",exact:true})).toBeVisible();
  const pane=page.locator(".issue-bridge-root .knowledge-list-pane");
  const visibleWidth=(await pane.boundingBox())!.width;
  await page.getByRole("button",{name:"Hide preview",exact:true}).click();
  await expect(page.locator(".issue-bridge-root")).toHaveAttribute("data-preview-hidden","true");
  const hiddenWidth=(await pane.boundingBox())!.width;
  expect(hiddenWidth).toBeGreaterThan(visibleWidth+100);
  expect(Math.abs(hiddenWidth-(await page.locator(".issue-list-shell").boundingBox())!.width), "hidden preview gives the board full width").toBeLessThan(2);
  await expect(page.locator(".knowledge-detail-pane")).not.toBeVisible();
  await page.getByRole("button",{name:"Show preview",exact:true}).click();
  await expect(page.locator(".issue-bridge-root")).toHaveAttribute("data-preview-hidden","false");
  await page.locator(".workspace-window").evaluate(element => { (element as HTMLElement).style.width="850px"; });
  for (const phase of ["backlog","queued","active","done"]) {
    expect((await column(page,phase).boundingBox())!.width).toBeGreaterThanOrEqual(262);
  }
  expect(await pane.evaluate(element=>element.scrollWidth>element.clientWidth)).toBe(true);
  await column(page,"done").scrollIntoViewIfNeeded();
  await expect(column(page,"done")).toBeVisible();
  await confirm(page,[]);
  await expect(column(page,"queued").locator(".knowledge-row")).toHaveCount(0);
  await column(page,"queued").scrollIntoViewIfNeeded();
  await expect(column(page,"queued")).toContainText("Nothing will launch until an issue is queued.");
});

test("T-7a: current labels explain themselves with keyboard focus and hover", async ({ page }) => {
  const popup = page.locator('.issue-control-explanation[role="tooltip"]');
  const controlMessages = () => page.evaluate(() => (window as any).__queueMessages.filter((message: any) =>
    message.kind.startsWith("set_issue_monitor_") || message.kind.startsWith("issue_monitor_queue_")));
  const before = await controlMessages();
  const queueMetric = page.locator('[data-metric="queue"]');
  await queueMetric.focus();
  await page.keyboard.press("Tab");
  const autonomous = page.getByRole("switch", { name: "Autonomous mode" });
  await expect(autonomous).toBeFocused();
  await expect(popup).toBeVisible();
  await expect(popup).toHaveText("Allow eligible issues to run without waiting for human approval.");
  await page.screenshot({ path: test.info().outputPath("issue-control-explanation.png") });
  await expect(autonomous).toHaveAttribute("aria-describedby", await popup.getAttribute("id") as string);
  await autonomous.hover();
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  await page.mouse.move(1590, 1090);
  await expect(popup).not.toBeVisible();
  await expect(autonomous).toBeFocused();

  for (const phase of ["backlog", "queued", "active", "done"]) {
    const heading = column(page, phase).locator(".issue-queue-heading");
    await heading.focus();
    await expect(heading).toBeFocused();
    await expect(popup).toBeVisible();
    await expect(popup).toHaveText(await heading.getAttribute("aria-description") as string);
  }
  const queued = column(page, "queued").locator(".issue-queue-heading");
  await queued.focus();
  await confirm(page, [3, 4, 1]);
  await expect(column(page, "queued").locator(".issue-queue-heading")).toHaveText("Queued · 3");
  await expect(column(page, "queued").locator(".issue-queue-heading")).toBeFocused();
  await expect(popup).toBeVisible();
  await page.keyboard.press("Escape");
  await confirm(page, [3, 4, 1, 2]);
  await expect(column(page, "queued").locator(".issue-queue-heading")).toHaveText("Queued · 4");
  await expect(column(page, "queued").locator(".issue-queue-heading")).toBeFocused();
  await expect(popup).not.toBeVisible();
  await expect(popup).toContainText("launch order");
  await page.locator(".knowledge-search").focus();
  await page.mouse.move(1590, 1090);
  await queued.hover();
  await expect(popup).toBeVisible();
  await expect(popup).toContainText("launch order");
  const box = await popup.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(1600);
  expect(box!.y + box!.height).toBeLessThanOrEqual(1100);
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await expect(popup).toBeVisible();
  await confirm(page, [3, 4]);
  await expect(queued).toHaveText("Queued · 2");
  await expect(popup).toBeVisible();
  await expect(queued).toHaveAttribute("aria-describedby", await popup.getAttribute("id") as string);
  await page.mouse.move(1590, 1090);
  await expect(popup).not.toBeVisible();
  await page.locator("#op-notifications-button").focus();
  await queued.hover();
  await expect(popup).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  for (const selector of [
    '.knowledge-monitor-pill', '[data-metric="active"]', '[data-metric="queue"]',
    '[data-action="monitor-auto-refill"]', '[data-action="monitor-auto-apply"]',
    '[data-action="toggle-issue-preview"]',
  ]) {
    const target = page.locator(selector);
    await target.focus();
    await expect(popup).toBeVisible();
    await expect(popup).toHaveText(await target.getAttribute("aria-description") as string);
  }
  const select = row(page, 4).locator(".knowledge-row-select");
  await select.focus();
  await expect(popup).toContainText("Auto-refill added this issue");
  expect(await controlMessages()).toEqual(before);
  await page.keyboard.press("Enter");
  const source = page.locator(".issue-detail-provenance");
  await source.focus();
  await expect(popup).toContainText("Auto-refill added this issue");
  await page.keyboard.press("Escape");
  await page.evaluate(() => (window as any).__queueRefreshDetail());
  await expect(page.locator(".issue-detail-title")).toHaveText("Refreshed issue");
  await expect(source).toBeFocused();
  await expect(popup).not.toBeVisible();
  await expect(popup).toContainText("Auto-refill added this issue");
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  await page.locator(".knowledge-search").focus();
  await row(page, 4).locator(".knowledge-row-badge").hover();
  await expect(popup).toBeVisible();
  const cardBox = await row(page, 4).boundingBox();
  const cardPopup = await popup.boundingBox();
  await page.mouse.move(cardBox!.x + 20, cardBox!.y + cardBox!.height - 1);
  await expect(popup).toBeVisible();
  await page.mouse.move(cardPopup!.x + 20, cardPopup!.y + 10);
  await expect(popup).toBeVisible();
  await source.focus();
  await expect(popup).toContainText("Auto-refill added this issue");
  await expect(popup).toBeVisible();
  await row(page, 5).locator(".knowledge-row-select").focus();
  await expect(popup).toHaveText("The agent is working on this issue.");
  await row(page, 5).locator(".knowledge-row-select").click();
  await page.getByRole("button", { name: "Output", exact: true }).click();
  for (const selector of [".issue-preview .knowledge-monitor-chip", ".issue-split-pair .knowledge-row-badge"]) {
    if (selector.startsWith(".issue-split")) await page.getByRole("button", { name: "Split", exact: true }).click();
    const badge = page.locator(selector);
    await badge.focus();
    await expect(badge).toBeFocused();
    await expect(popup).toHaveText("The agent is working on this issue.");
    await badge.hover();
    await expect(popup).toBeVisible();
    await page.keyboard.press("Escape");
    await page.mouse.move(1590, 1090);
    await expect(popup).not.toBeVisible();
  }
});


test("card selection keeps issue body, acceptance and provenance beside switchable output", async ({page}) => {
  const detail=page.locator(".knowledge-detail-pane");
  for (const [number,source] of [[3,"Operator"],[4,"Auto-refill"]] as const) {
    await row(page,number).locator(".knowledge-row-select").click();
    await expect(page.getByRole("button",{name:"Issue",exact:true})).toHaveAttribute("aria-pressed","true");
    await expect(detail.getByText(`Description for #${number}`, {exact:true})).toBeVisible();
    await detail.locator(".knowledge-section").filter({hasText:"Acceptance criteria"}).locator("summary").click();
    await expect(detail.getByText(`AC-${number}: expected behavior`, {exact:true})).toBeVisible();
    await expect(detail.locator(".issue-detail-provenance")).toHaveText(`Queued by: ${source}`);
  }
  await page.getByRole("button",{name:"Output",exact:true}).click();
  await expect(detail.locator(".issue-preview-empty")).toHaveText("Waiting in queue. No agent has started.");
  await row(page,6).locator(".knowledge-row-select").click();
  await expect(page.getByRole("button",{name:"Output",exact:true})).toHaveAttribute("aria-pressed","true");
  await expect(detail.locator(".issue-preview-empty")).toHaveText("This issue is completed. No agent is running.");
  await page.getByRole("button",{name:"Issue",exact:true}).click();
  await expect(detail).toContainText("Description for #6");
  await expect(page.locator(".issue-queue-board")).toBeVisible();
});

test("urgent priority, cap fallback and PM demotion preserve assignment provenance", async ({page}) => {
  await confirm(page, [3, 4, 1], { terminal_queue: [
    {number: 3, queued_by: "operator", priority: "urgent", priority_reason: "urgent_label", assigned_by: "alice", assigned_at: "2026-10-03T01:00:00Z"},
    {number: 4, queued_by: "auto-refill", priority: "normal", priority_reason: "urgent_limit_reached"},
    {number: 1, queued_by: "operator", priority: "normal", priority_reason: "pm_demoted", assigned_by: "pm-session", assigned_at: "2026-10-03T02:00:00Z"},
  ]});
  await expect.poll(() => column(page, "queued").locator(".knowledge-row").evaluateAll(rows => rows.map(row => row.getAttribute("data-issue-number")))).toEqual(["3", "4", "1"]);
  for (const [number, label, actor, time] of [
    [3, "Urgent", "alice", "2026-10-03T01:00:00Z"],
    [4, "Normal · Urgent limit reached", "Unknown", "Not observed"],
    [1, "Normal · PM demoted", "pm-session", "2026-10-03T02:00:00Z"],
  ] as const) {
    const card = row(page, number);
    const priority = card.locator('[data-key="queue-priority"]');
    await expect(priority).toHaveText(label);
    const cardBounds = (await card.boundingBox())!;
    const priorityBounds = (await priority.boundingBox())!;
    expect(priorityBounds.x + priorityBounds.width, "priority chip fits within queued card").toBeLessThanOrEqual(cardBounds.x + cardBounds.width);
    expect(await priority.evaluate(element => element.scrollWidth <= element.clientWidth), "priority text fits without overflowing").toBe(true);
    await row(page, number).locator(".knowledge-row-select").click();
    await expect(page.locator(".issue-detail-priority")).toHaveText(`Priority: ${label}`);
    await expect(page.locator(".issue-detail-priority-assignment")).toHaveText(`Priority assigned by: ${actor} · Assigned at: ${time}`);
  }
});

test("bulk drag and reorder wait for confirmation, forbidden and rejected changes retain cards", async ({ page }) => {
  await row(page,1).getByRole("checkbox").check();
  await row(page,2).getByRole("checkbox").check();
  await drag(page,1,column(page,"queued").locator("h3"));
  await expect.poll(()=>messages(page)).toEqual([{kind:"issue_monitor_queue_push",issue_numbers:[1,2]}]);
  await expect(column(page,"backlog").locator('[data-issue-number="1"]')).toHaveCount(1);
  await confirm(page,[3,4,1,2]);
  await expect(column(page,"queued").locator(".knowledge-row")).toHaveCount(4);
  await row(page,1).getByRole("checkbox").uncheck();
  await row(page,2).getByRole("checkbox").uncheck();
  await drag(page,2,row(page,3));
  await expect.poll(async()=>(await messages(page)).at(-1)).toEqual({kind:"issue_monitor_queue_move",issue_number:2,position:0});
  await expect(column(page,"queued").locator(".knowledge-row").first()).toHaveAttribute("data-issue-number","3");
  await confirm(page,[2,3,4,1]);
  await expect(column(page,"queued").locator(".knowledge-row").first()).toHaveAttribute("data-issue-number","2");
  await drag(page,2,column(page,"backlog").locator("h3"));
  await expect.poll(async()=>(await messages(page)).at(-1)).toEqual({kind:"issue_monitor_queue_remove",issue_numbers:[2]});
  await confirm(page,[3,4,1]);
  await expect(column(page,"backlog").locator('[data-issue-number="2"]')).toHaveCount(1);
  const before=(await messages(page)).length;
  for(const phase of ["active","done"]) {
    await drag(page,2,column(page,phase).locator("h3"));
    await expect(page.locator(".issue-queue-feedback")).toContainText("controlled by the monitor");
  }
  expect((await messages(page)).length).toBe(before);
  await drag(page,2,column(page,"queued").locator("h3"));
  await confirm(page,[3,4,1],{last_error:"Queue change rejected: issue is on hold"});
  await expect(column(page,"backlog").locator('[data-issue-number="2"]')).toHaveCount(1);
  await page.locator("#op-notifications-button").click();
  await expect(page.locator("#notification-center")).toContainText("Queue change rejected: issue is on hold");
});

test("auto-refill starts off and toggle and limit use server-confirmed values", async ({ page }) => {
  const toggle=page.getByRole("switch",{name:"Auto-refill queue",exact:true});
  await expect(toggle).toHaveAttribute("aria-checked","false");
  await toggle.click();
  await expect.poll(()=>messages(page)).toEqual([{kind:"set_issue_monitor_auto_refill",enabled:true,limit:3}]);
  await expect(toggle).toHaveAttribute("aria-checked","false");
  await confirm(page,[3,4],{terminal_queue_auto_refill:true});
  await expect(toggle).toHaveAttribute("aria-checked","true");
  await page.getByRole("spinbutton",{name:"Auto-refill queue limit",exact:true}).fill("5");
  await page.getByRole("spinbutton",{name:"Auto-refill queue limit",exact:true}).press("Tab");
  await expect.poll(async()=>(await messages(page)).at(-1)).toEqual({kind:"set_issue_monitor_auto_refill",enabled:true,limit:5});
  await confirm(page,[3,4],{terminal_queue_auto_refill_limit:5});
  await expect(page.getByRole("spinbutton",{name:"Auto-refill queue limit",exact:true})).toHaveValue("5");
});

test("live launched state survives stale cache and retains full queue positions", async ({ page }) => {
  await confirm(page, [99, 3, 4]);
  await page.evaluate(() => (window as any).__queueInbox([{issue:{number:3},state:"launched"}]));
  await expect(column(page, "queued").locator('[data-issue-number="3"]')).toHaveCount(0);
  await expect(column(page, "active").locator('[data-issue-number="3"]')).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__queueMessages.filter((message:any) =>
    message.kind === "load_knowledge_bridge").length)).toBeGreaterThan(1);
  await expect(column(page, "active").locator('[data-issue-number="3"]')).toBeVisible();
  const move = row(page, 4).locator('[data-action="move-up"]');
  if (!await move.isVisible()) await row(page, 4).locator('.knowledge-row-menu summary').click();
  await move.click();
  await expect.poll(async () => (await messages(page)).at(-1)).toEqual({kind:"issue_monitor_queue_move",issue_number:4,position:1});
});

test("an unfocused Issue window reflects a launch while animation frames are suspended", async ({ page }, info) => {
  await page.evaluate(() => (window as any).__queueAddForeground());
  await expect(page.locator('[data-id="tab-queue::console"]')).toHaveClass(/focused/);
  await expect(page.locator('[data-id="tab-queue::issue-1"]')).not.toHaveClass(/focused/);
  const elapsed = await page.evaluate(async () => {
    const scheduleFrame = window.requestAnimationFrame;
    window.requestAnimationFrame = () => 0;
    const start = performance.now();
    const rendered = new Promise<number>(resolve => {
      const observer = new MutationObserver(() => {
        if (!document.querySelector('[data-queue-column="active"] [data-issue-number="3"]')) return;
        observer.disconnect();
        resolve(performance.now() - start);
      });
      observer.observe(document.body, {childList:true,subtree:true});
    });
    (window as any).__queueInbox([{issue:{number:3},state:"launched"}]);
    try { return await rendered; } finally { window.requestAnimationFrame = scheduleFrame; }
  });
  expect(elapsed, 'model-to-view update without a focus or animation frame').toBeLessThanOrEqual(1000);
  await info.attach('unfocused-render-timing', {body:JSON.stringify({elapsed_ms:elapsed,limit_ms:1000}),contentType:'application/json'});
  await expect(page.locator('[data-id="tab-queue::issue-1"]')).not.toHaveClass(/focused/);
  await expect(column(page, "queued").locator('[data-issue-number="3"]')).toHaveCount(0);
});

async function installBackend(page: Page, { includeAgent = false, observeSkipped = false } = {}) {
  const projectKey=new URL(liveUrl||APP_URL).pathname.split("/")[2];
  await page.addInitScript(({projectKey, includeAgent, observeSkipped})=>{
    const fixture=window as any;
    fixture.__queueMessages=[];
    const status:any={enabled:false,state:"disabled",active_count:1,max_active_agents:1,
      launch_profile_source:"saved",launch_profile_summary:"Fixture agent",terminal_queue_auto_refill:false,
      terminal_queue_auto_refill_limit:3,terminal_queue:[{number:3,queued_by:"operator"},{number:4,queued_by:"auto-refill"}],queue_len:2};
    const entries=[
      {number:1,title:"First backlog issue",...(observeSkipped ? {labels:["gwt-queued"],monitor_state:"skipped",
        exclusion_reason:"not selected in this terminal queue"} : {})},{number:2,title:"Second backlog issue"},
      {number:3,title:"Operator queued issue",monitor_state:"queued",queue_position:1,queued_by:"operator"},
      {number:4,title:"Auto-refilled issue",monitor_state:"queued",queue_position:2,queued_by:"auto-refill"},
      {number:5,title:"Running issue",monitor_state:"launched"},{number:6,title:"Completed issue",state:"closed"},
    ].map(entry=>({state:"open",labels:[],is_spec:false,linked_branch_count:0,related_work_refs:[],...entry}));
    class FixtureSocket extends EventTarget {
      static CONNECTING=0; static OPEN=1; static CLOSING=2; static CLOSED=3;
      readyState=0;
      constructor(public readonly url:string) { super(); fixture.__queueConfirm=(numbers:number[],extra:any)=>{
        Object.assign(status,{terminal_queue:numbers.map(number=>({number,queued_by:number===4?"auto-refill":"operator"})),queue_len:numbers.length},extra);
        this.emit({kind:"issue_monitor_status",status});
      }; fixture.__queueAddForeground=()=>{
        const workspace=fixture.__queueWorkspace;
        workspace.tabs[0].workspace.windows.push({id:"tab-queue::console",title:"Console",preset:"console",
          geometry:{x:1000,y:80,width:500,height:500},z_index:2,status:"running",persist:true,minimized:false,maximized:false});
        this.emit({kind:"workspace_state",workspace});
      }; fixture.__queueInbox=(items:unknown[])=>this.dispatchEvent(new MessageEvent("message",{
        data:JSON.stringify({kind:"issue_monitor_inbox",items})
      })); fixture.__queueRefreshDetail=()=>this.emit({...fixture.__queueLastDetail,
        detail:{...fixture.__queueLastDetail.detail,title:"Refreshed issue"}});
        setTimeout(()=>{this.readyState=1;this.dispatchEvent(new Event("open"));},0); }
      emit(payload:unknown) {const data=JSON.stringify(payload);setTimeout(()=>this.dispatchEvent(new MessageEvent("message",{data})),0);}
      send(raw:string) {
        const message=JSON.parse(raw);fixture.__queueMessages.push(message);
        if(message.kind==="frontend_ready") { fixture.__queueWorkspace={app_version:"playwright",tabs:[{
          id:"tab-queue",title:"Queue fixture",project_root:"/fixture",project_key:projectKey,kind:"git",
          workspace:{viewport:{x:0,y:0,zoom:1},windows:[{id:"tab-queue::issue-1",title:"Issues",preset:"issue",
            geometry:{x:40,y:40,width:1470,height:950},z_index:1,status:"running",persist:true,minimized:false,maximized:false},
            ...(includeAgent ? [{id:"tab-queue::agent-5",session_id:"agent-5-session",title:"Issue 5 agent",preset:"agent",agent_id:"codex",
              geometry:{x:80,y:80,width:900,height:600},z_index:2,status:"running",persist:true,minimized:false,maximized:false,
              placement:{kind:"issue_preview",issue_window_id:"tab-queue::issue-1",issue_number:5}}] : [])]} }],active_tab_id:"tab-queue",recent_projects:[]};
          this.emit({kind:"workspace_state",workspace:fixture.__queueWorkspace}); }
        else if(message.kind==="list_issue_monitor") this.emit({kind:"issue_monitor_status",status});
        else if(["load_knowledge_bridge","search_knowledge_bridge"].includes(message.kind)) this.emit({kind:"knowledge_entries",id:message.id,knowledge_kind:"issue",request_id:message.request_id,entries,selected_number:null,refresh_enabled:true});
        else if(message.kind==="select_knowledge_bridge_entry") {
          fixture.__queueLastDetail={kind:"knowledge_detail",id:message.id,knowledge_kind:"issue",request_id:message.request_id,detail:{number:message.number,title:entries.find(e=>e.number===message.number)?.title,state:message.number===6?"closed":"open",labels:[],sections:[{title:"Description",body:`Description for #${message.number}`,body_html:`<p>Description for #${message.number}</p>`},{title:"Acceptance criteria",body:`AC-${message.number}: expected behavior`,body_html:`<p>AC-${message.number}: expected behavior</p>`}],related_works:[]}};
          this.emit(fixture.__queueLastDetail);
        }
      }
      close(){this.readyState=3;this.dispatchEvent(new CloseEvent("close"));}
    }
    Object.defineProperty(window,"WebSocket",{configurable:true,value:FixtureSocket});
  },{projectKey, includeAgent, observeSkipped});
}
