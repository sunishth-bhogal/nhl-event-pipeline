// Breakaway trade page: live market on a replayed game, served by the Rust
// exchange server (engine/src/bin/server). Market data arrives as
// Server-Sent Events; orders go to the JSON API.
(() => {
  "use strict";
  const $ = (id) => document.getElementById(id);
  const { teamColors } = window.BreakawayTeams;
  const store = {
    get(k) { try { return localStorage.getItem(k); } catch (_) { return null; } },
    set(k, v) { try { localStorage.setItem(k, v); } catch (_) { /* private mode */ } },
    del(k) { try { localStorage.removeItem(k); } catch (_) { /* private mode */ } },
  };

  let model = null, game = null, market = null, token = null;
  let sec = 0, status = "live", fair = 50, last = -1, bids = [], asks = [];
  let mids = [], trades = [], samples = [], events = [];
  let side = "buy", me = null, lastScore = null;

  const money = (c) => `${c < 0 ? "−" : ""}$${(Math.abs(c) / 100).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
  const mmss = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

  async function api(method, path, body) {
    const headers = { "Content-Type": "application/json" };
    if (token) headers.Authorization = `Bearer ${token}`;
    const r = await fetch(path, { method, headers, body: body ? JSON.stringify(body) : undefined });
    let data = {};
    try { data = await r.json(); } catch (_) { /* empty body */ }
    return { status: r.status, data };
  }

  // ---------------------------------------------------------------- game
  function clockAt(s, period) {
    if (s < 3600 || (s === 3600 && period <= 3)) {
      const p = Math.min(3, Math.floor(s / 1200) + 1);
      return { label: ["1st", "2nd", "3rd"][p - 1], clock: mmss(p * 1200 - s) };
    }
    if (game.type === 2) return period >= 5 ? { label: "Shootout", clock: "SO" } : { label: "Overtime", clock: mmss(Math.max(0, 3900 - s)) };
    const n = Math.floor((s - 3600) / 1200) + 1;
    return { label: n === 1 ? "Overtime" : `${n}OT`, clock: mmss(1200 - ((s - 3600) % 1200)) };
  }

  async function loadGame(id) {
    game = await fetch(`games/${id}.json`).then((r) => r.json());
    const { h, a } = teamColors(game.home.abbrev, game.away.abbrev);
    document.documentElement.style.setProperty("--home", h);
    document.documentElement.style.setProperty("--away", a);
    $("homeAbbrev").textContent = game.home.abbrev;
    $("awayAbbrev").textContent = game.away.abbrev;
    $("homeName").textContent = game.home.name;
    $("awayName").textContent = game.away.name;
    $("tkTeam").textContent = game.home.abbrev;
    $("buyHint").textContent = `${game.home.abbrev} wins`;
    $("sellHint").textContent = `${game.home.abbrev} doesn't win`;
    $("modelLab").textContent = `model: ${game.home.abbrev} wins`;
    // model line: the same win-probability model as the replay page
    let st = { hs: 0, as: 0, sit: "1551", so: false, final: false, period: 1 }, tie = 0;
    const wp = (s, x) => WP.predict(model, s, x.hs, x.as, x.sit, game.elo_diff, x.so, x.final);
    samples = [[0, wp(0, st)]];
    events = [];
    for (const [s, period, kind, owner, sit, hs, as] of game.plays) {
      for (let k = Math.ceil((samples[samples.length - 1][0] + 1) / 15) * 15; k < s; k += 15) samples.push([k, wp(k, st)]);
      const so = game.type === 2 && period >= 5, final = kind === "END" || kind === "SOC";
      if (!so && !final) tie = hs;
      const before = wp(s, st);
      st = { hs: final ? game.final.home : so ? tie : hs, as: final ? game.final.away : so ? tie : as, sit: sit || st.sit, so, final, period };
      samples.push([s, before], [s, wp(s, st)]);
      events.push({ s, period, kind, owner, st });
    }
  }

  function stateAt(s) {
    let e = null;
    for (const x of events) { if (x.s > s) break; e = x; }
    return e ? e.st : { hs: 0, as: 0, period: 1, final: false, so: false };
  }

  // ---------------------------------------------------------------- stream
  function connect() {
    const es = new EventSource("api/stream");
    es.onopen = () => { $("conn").textContent = "Live"; $("conn").className = "brand-tag brand-tag--live is-on"; };
    es.onerror = () => { $("conn").textContent = "Reconnecting…"; $("conn").className = "brand-tag brand-tag--live is-off"; };
    es.onmessage = async (m) => {
      const d = JSON.parse(m.data);
      if (d.market !== market) {
        market = d.market;
        mids = []; trades = []; me = null; lastScore = null;
        await loadGame(market);
        token = store.get(`bk-token-${market}`);
        refreshMe();
      }
      if (d.type === "tick") {
        sec = d.sec; status = d.status; fair = d.fair; last = d.last; bids = d.bids; asks = d.asks;
        if (bids.length && asks.length) mids.push([sec, (bids[0][0] + asks[0][0]) / 2]);
        for (const t of d.trades) trades.push(t);
        if (trades.length > 60) trades = trades.slice(-60);
        render();
      } else if (d.type === "market") {
        sec = d.sec; status = d.status;
        render();
      }
    };
  }

  // ---------------------------------------------------------------- account
  async function refreshMe() {
    if (!token) { me = null; renderAccount(); return; }
    const r = await api("GET", "api/me");
    if (r.status === 401) { store.del(`bk-token-${market}`); token = null; me = null; }
    else if (r.status === 200) me = r.data;
    renderAccount();
  }

  async function join() {
    $("joinBtn").disabled = true;
    const r = await api("POST", "api/join");
    $("joinBtn").disabled = false;
    if (r.status === 201) {
      token = r.data.token;
      store.set(`bk-token-${r.data.market}`, token);
      await refreshMe();
    } else {
      $("joinBox").querySelector(".hint").textContent = r.data.error || "Couldn't join right now.";
    }
  }

  function describe(evs) {
    let filled = 0, notional = 0, rested = 0, why = "";
    for (const e of evs) {
      if (e.type === "fill") { filled += e.qty; notional += e.qty * e.price; }
      if (e.type === "rested") rested = e.qty;
      if (e.type === "rejected") why = e.reason;
    }
    if (why) return { ok: false, text: `Rejected: ${why}.` };
    const parts = [];
    if (filled) parts.push(`Filled ${filled} @ ${(notional / filled).toFixed(1)}¢ avg`);
    if (rested) parts.push(`${rested} resting in the book`);
    if (!filled && !rested) parts.push("Nothing to match at that price; cancelled");
    return { ok: true, text: parts.join(" · ") + "." };
  }

  async function place(ev) {
    ev.preventDefault();
    const price = +$("price").value, qty = +$("qty").value;
    const msg = $("msg");
    if (!(price >= 1 && price <= 99 && Number.isInteger(price))) { msg.className = "msg bad"; msg.textContent = "Price must be a whole number from 1 to 99."; return; }
    if (!(qty >= 1 && Number.isInteger(qty))) { msg.className = "msg bad"; msg.textContent = "Contracts must be a whole number, at least 1."; return; }
    $("placeBtn").disabled = true;
    const r = await api("POST", "api/orders", { side, price, qty, tif: $("ioc").checked ? "ioc" : "gtc" });
    $("placeBtn").disabled = false;
    if (r.status === 401) { token = null; store.del(`bk-token-${market}`); renderAccount(); return; }
    const d = r.data.events ? describe(r.data.events) : { ok: false, text: r.data.error || "Something went wrong." };
    msg.className = `msg ${d.ok ? "ok" : "bad"}`;
    msg.textContent = d.text;
    refreshMe();
  }

  async function cancel(id) {
    await api("DELETE", `api/orders/${id}`);
    refreshMe();
  }

  // ---------------------------------------------------------------- render
  function updatePreview() {
    const price = +$("price").value, qty = +$("qty").value, home = game ? game.home.abbrev : "home";
    if (!(price >= 1 && price <= 99 && qty >= 1)) { $("preview").textContent = ""; return; }
    if (side === "buy") {
      $("preview").innerHTML = `Costs up to <b>${money(price * qty)}</b>. Pays <b>${money(100 * qty)}</b> if ${home} wins.`;
    } else {
      const held = me ? Math.min(me.yes, qty) : 0;
      const cash = (qty - held) * (100 - price);
      $("preview").innerHTML = held
        ? `Sells up to ${held} ${home}-wins shares you hold${qty > held ? `, and sets aside <b>${money(cash)}</b> for the rest` : ""}.`
        : `Sets aside up to <b>${money(cash)}</b>. Pays <b>${money(100 * qty)}</b> if ${home} doesn't win.`;
    }
  }

  function renderAccount() {
    const joined = !!(token && me);
    const over = status === "final";
    $("joinBox").hidden = joined;
    $("ticket").hidden = !joined;
    $("placeBtn").disabled = over;
    $("joinBtn").disabled = over;
    if (over && !joined) $("joinBox").querySelector(".hint").textContent = "This game is over. The next one starts in a few seconds.";
    if (!joined) {
      $("acct").innerHTML = "<div><dt>Status</dt><dd>Not joined</dd></div>";
      $("orders").innerHTML = '<li class="empty">None</li>';
      return;
    }
    const pnl = me.pnl_marked;
    $("acct").innerHTML = [
      ["Cash", money(me.cash)],
      ["Set aside for orders", money(me.cash_reserved)],
      [`${game.home.abbrev} wins shares`, me.yes],
      [`${game.home.abbrev} doesn't-win shares`, me.no],
      [over ? "Final P&L" : "P&L at model price", money(pnl), pnl > 0 ? "pos" : pnl < 0 ? "neg" : ""],
    ].map(([k, v, c]) => `<div><dt>${k}</dt><dd class="${c || ""}">${v}</dd></div>`).join("");
    $("orders").innerHTML = me.orders.length
      ? me.orders.map((o) => `<li><span>${o.side === "buy" ? "Buy" : "Sell"} ${o.qty} @ ${o.price}¢</span><button type="button" data-cancel="${o.id}">Cancel</button></li>`).join("")
      : '<li class="empty">None</li>';
    updatePreview();
  }

  function render() {
    if (!game) return;
    const st = status === "final" ? { hs: game.final.home, as: game.final.away, period: 3, final: true } : stateAt(sec);
    const score = `${st.as}-${st.hs}`;
    if (lastScore !== null && score !== lastScore) {
      const light = $("goalLight");
      light.classList.remove("on"); void light.offsetWidth; light.classList.add("on");
    }
    lastScore = score;
    $("homeScore").textContent = st.hs;
    $("awayScore").textContent = st.as;
    if (status === "final") {
      $("period").textContent = "Final";
      $("clock").textContent = `${game.final.away}–${game.final.home}`;
      $("status").textContent = `${game.final.home > game.final.away ? game.home.abbrev : game.away.abbrev} wins`;
    } else {
      const c = clockAt(sec, st.period);
      $("period").textContent = c.label;
      $("clock").textContent = c.clock;
      $("status").textContent = "Live replay";
    }
    $("lastPx").textContent = last >= 0 ? `${last}¢` : "—";
    $("modelPx").textContent = `${Math.round(fair)}¢`;
    $("oddsFill").style.width = `${fair}%`;
    drawChart();
    // book
    const maxQ = Math.max(1, ...bids.map((l) => l[1]), ...asks.map((l) => l[1]));
    const row = (cls, [p, q]) => `<li class="${cls}" tabindex="0" data-px="${p}" data-side="${cls === "ask" ? "buy" : "sell"}"><span class="bar" style="width:${(q / maxQ) * 100}%"></span><span class="px">${p}¢</span><span class="qty">${q.toLocaleString()}</span></li>`;
    $("ladder").innerHTML = asks.slice(0, 6).reverse().map((l) => row("ask", l)).join("")
      + `<li class="spread">${bids.length && asks.length ? `${asks[0][0] - bids[0][0]}¢ spread` : "one-sided"}</li>`
      + bids.slice(0, 6).map((l) => row("bid", l)).join("");
    // trades
    const recent = trades.slice(-10).reverse();
    $("tape").innerHTML = recent.length
      ? recent.map(([s, p, q, buy]) => {
          const c = clockAt(s, s >= 3600 ? 4 : 1);
          return `<li><span class="t">${c.label.replace("Overtime", "OT")} ${c.clock}</span><span class="${buy ? "buy" : "sell"}">${buy ? "Buy" : "Sell"}</span><span>${q} @ ${p}¢</span><span></span></li>`;
        }).join("")
      : '<li class="empty" style="background:none">Waiting for trades…</li>';
    if (status === "final") renderAccount();
  }

  function drawChart() {
    const svg = $("chart");
    const W = svg.clientWidth, H = svg.clientHeight;
    if (!W || !H || !samples.length) return;
    const pl = 8, pr = 44, pt = 10, pb = 22;
    const xMax = sec <= 3600 ? 3600 : game.type === 2 ? 3900 : 3600 + 1200 * Math.ceil((sec - 3600) / 1200);
    const x = (s) => pl + (Math.min(s, xMax) / xMax) * (W - pl - pr);
    const y = (p) => pt + (1 - p) * (H - pt - pb);
    let g = "";
    for (const p of [0.25, 0.5, 0.75]) g += `<line class="${p === 0.5 ? "c-mid" : "c-grid"}" x1="${x(0)}" x2="${x(xMax)}" y1="${y(p)}" y2="${y(p)}"/>`;
    for (const b of [1200, 2400, 3600]) if (b < xMax) g += `<line class="c-grid" x1="${x(b)}" x2="${x(b)}" y1="${y(1)}" y2="${y(0)}"/>`;
    ["1st", "2nd", "3rd"].forEach((n, i) => { g += `<text class="c-axis" x="${x(i * 1200 + 600)}" y="${H - 5}" text-anchor="middle">${n}</text>`; });
    g += `<text class="c-team" x="${W - pr + 8}" y="${y(1) + 10}" fill="var(--home)">99¢</text><text class="c-team" x="${W - pr + 8}" y="${y(0) - 2}" fill="var(--away)">1¢</text>`;
    const pts = samples.filter((p) => p[0] <= sec);
    const line = pts.map((p, i) => `${i ? "L" : "M"}${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`).join("") + `L${x(sec).toFixed(1)},${y(fair / 100).toFixed(1)}`;
    if (mids.length > 1) g += `<path class="c-market" d="${mids.map((m, i) => `${i ? "L" : "M"}${x(m[0]).toFixed(1)},${y(m[1] / 100).toFixed(1)}`).join("")}"/>`;
    g += `<path class="c-line" d="${line}"/>`;
    g += `<circle class="c-head" cx="${x(sec)}" cy="${y(fair / 100)}" r="5"/>`;
    svg.innerHTML = g;
  }

  // ---------------------------------------------------------------- wiring
  function setSide(s) {
    side = s;
    document.querySelectorAll(".sd").forEach((b) => {
      const on = b.dataset.side === s;
      b.classList.toggle("on", on);
      b.setAttribute("aria-checked", String(on));
    });
    $("placeBtn").textContent = s === "buy" ? "Place buy order" : "Place sell order";
    updatePreview();
  }

  function bind() {
    $("joinBtn").addEventListener("click", join);
    $("ticket").addEventListener("submit", place);
    document.querySelectorAll(".sd").forEach((b) => b.addEventListener("click", () => setSide(b.dataset.side)));
    ["price", "qty"].forEach((id) => $(id).addEventListener("input", updatePreview));
    const pick = (li) => {
      if (!li || !li.dataset.px) return;
      $("price").value = li.dataset.px;
      setSide(li.dataset.side);
      $("price").focus();
    };
    $("ladder").addEventListener("click", (e) => pick(e.target.closest("li")));
    $("ladder").addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); pick(e.target.closest("li")); } });
    $("orders").addEventListener("click", (e) => { const b = e.target.closest("[data-cancel]"); if (b) cancel(b.dataset.cancel); });
    new ResizeObserver(drawChart).observe($("fig"));
    setInterval(() => { if (token && status !== "final") refreshMe(); }, 1500);
    setSide("buy");
  }

  (async function init() {
    model = await fetch("wp_model.json").then((r) => r.json());
    bind();
    connect();
  })().catch((e) => { $("why").textContent = "Couldn't start the trading page."; console.error(e); });
})();
