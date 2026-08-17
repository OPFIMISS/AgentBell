const $ = (id) => document.getElementById(id)
const state = {
  token: localStorage.getItem('agentbell_token') || '',
  data: null,
  ws: null,
  reconnect: 0,
  selecting: false,
  selectedEvents: new Set()
}
const pairSecret = new URLSearchParams(location.search).get('pair') || ''

function esc(value = '') {
  return String(value).replace(/[&<>'"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', "'": '&#39;', '"': '&quot;' })[c])
}

function toast(message) {
  $('toast').textContent = message
  $('toast').classList.add('show')
  clearTimeout(toast.timer)
  toast.timer = setTimeout(() => $('toast').classList.remove('show'), 3200)
}

async function api(path, options = {}) {
  const headers = { ...(options.headers || {}) }
  if (state.token) headers.Authorization = `Bearer ${state.token}`
  if (options.body && typeof options.body !== 'string') {
    headers['Content-Type'] = 'application/json'
    options.body = JSON.stringify(options.body)
  }
  const response = await fetch(path, { ...options, headers })
  if (!response.ok) throw new Error((await response.text()) || `HTTP ${response.status}`)
  return response.status === 204 ? null : response.json()
}

async function load() {
  try {
    const data = await api(`/api/status?token=${encodeURIComponent(state.token)}`)
    state.data = data
    $('loading').classList.add('hidden')
    if (!data.authorized) {
      $('pairView').classList.remove('hidden')
      $('appView').classList.add('hidden')
      if (!pairSecret) $('pairHint').textContent = '请在电脑端扫描二维码，或等待电脑批准本次连接。'
      return
    }
    $('pairView').classList.add('hidden')
    $('appView').classList.remove('hidden')
    $('appView').classList.toggle('mobile', !data.admin)
    $('adminRail').classList.toggle('hidden', !data.admin)
    document.querySelectorAll('.admin-only').forEach(el => el.classList.toggle('hidden', !data.admin))
    $('serverName').textContent = data.device_name
    render(data)
    connectWebSocket()
  } catch (error) {
    $('liveText').textContent = '连接失败'
    $('loading').innerHTML = `<div class="empty-state"><i>!</i><strong>无法连接电脑</strong><p>${esc(error.message)}</p></div>`
  }
}

async function pairDevice() {
  const button = $('pairButton')
  button.disabled = true
  button.textContent = '正在授权'
  try {
    const result = await api('/api/pair', { method: 'POST', body: { name: $('deviceName').value.trim() || '我的手机', pair_secret: pairSecret || null } })
    if (result.state === 'trusted') {
      state.token = result.token
      localStorage.setItem('agentbell_token', state.token)
      history.replaceState({}, '', '/')
      toast('已连接，可以接收通知')
      await requestNotifications(false)
      await load()
    } else {
      state.token = result.token
      localStorage.setItem('agentbell_token', state.token)
      $('pairHint').textContent = `已发送请求。请在电脑确认配对码 ${result.code}`
      button.textContent = '等待电脑批准'
      setTimeout(load, 2500)
    }
  } catch (error) {
    $('pairHint').textContent = error.message
    button.disabled = false
    button.textContent = '重新连接'
  }
}

function render(data) {
  renderStats(data.events)
  renderEvents(data.events)
  if (data.admin) {
    $('qrCode').innerHTML = data.pair_svg || '<span>未找到可用局域网地址</span>'
    $('pairUrl').textContent = data.pair_url || ''
    renderDevices(data.devices, data.pending, data.discovered || [])
    renderAdapters(data.adapters)
  }
  updateNotifyButton()
}

function renderStats(events) {
  const today = new Date().toDateString()
  const todayEvents = events.filter(e => new Date(e.timestamp_ms).toDateString() === today)
  const completed = todayEvents.filter(e => e.kind === 'completed').length
  const attention = todayEvents.filter(e => ['failed', 'needs_input', 'approval_required'].includes(e.kind)).length
  $('stats').innerHTML = `<div class="stat"><span>今日通知</span><strong>${todayEvents.length}</strong></div><div class="stat"><span>完成任务</span><strong>${completed}</strong></div><div class="stat"><span>需要处理</span><strong>${attention}</strong></div>`
}

function renderEvents(events) {
  const currentIds = new Set(events.map(event => event.id))
  state.selectedEvents.forEach(id => {
    if (!currentIds.has(id)) state.selectedEvents.delete(id)
  })
  if (!events.length) state.selecting = false
  $('eventCount').textContent = events.length
  updateSelectionUi(events)
  if (!events.length) {
    $('eventList').innerHTML = '<div class="empty-state"><i>✓</i><strong>通知链路已经就绪</strong><p>Agent 完成任务后会出现在这里</p></div>'
    return
  }
  $('eventList').innerHTML = events.map(eventCard).join('')
  bindEventSelection()
}

function eventCard(event) {
  const symbol = { completed: '✓', failed: '!', needs_input: '?', approval_required: '→', started: '•' }[event.kind] || '•'
  const kindText = { completed: '已完成', failed: '失败', needs_input: '等待回复', approval_required: '等待批准', started: '已开始' }[event.kind] || event.kind
  const time = new Date(event.timestamp_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  const selected = state.selectedEvents.has(event.id)
  const checkbox = state.selecting ? `<label class="event-check" aria-label="选择 ${esc(event.title)}"><input type="checkbox" data-event-select="${esc(event.id)}" ${selected ? 'checked' : ''}></label>` : ''
  return `<article class="event-card ${esc(event.kind)}${state.selecting ? ' selecting' : ''}${selected ? ' selected' : ''}" data-event-id="${esc(event.id)}" aria-selected="${selected}">${checkbox}<div class="event-icon">${symbol}</div><div class="event-main"><strong>${esc(event.title)}</strong><p>${esc(event.message)}${event.project ? ` · ${esc(event.project)}` : ''}</p></div><div class="event-meta"><b>${esc(event.agent)}</b>${kindText} · ${time}</div></article>`
}

function updateSelectionUi(events = state.data?.events || []) {
  const admin = Boolean(state.data?.admin)
  $('selectButton').classList.toggle('hidden', !admin || !events.length)
  $('selectButton').textContent = state.selecting ? '取消' : '选择'
  $('selectionToolbar').classList.toggle('hidden', !admin || !state.selecting)
  $('selectedCount').textContent = `已选 ${state.selectedEvents.size} 项`
  const allSelected = events.length > 0 && state.selectedEvents.size === events.length
  $('selectAllButton').textContent = allSelected ? '取消全选' : '全选'
  $('deleteSelectedButton').disabled = state.selectedEvents.size === 0
}

function setSelectionMode(enabled) {
  state.selecting = enabled
  if (!enabled) state.selectedEvents.clear()
  renderEvents(state.data?.events || [])
}

function toggleEventSelection(id, force) {
  const selected = force ?? !state.selectedEvents.has(id)
  if (selected) state.selectedEvents.add(id)
  else state.selectedEvents.delete(id)
  renderEvents(state.data?.events || [])
}

function bindEventSelection() {
  document.querySelectorAll('[data-event-select]').forEach(input => {
    input.addEventListener('click', event => event.stopPropagation())
    input.addEventListener('change', () => toggleEventSelection(input.dataset.eventSelect, input.checked))
  })
  document.querySelectorAll('[data-event-id]').forEach(card => {
    let timer
    let startX = 0
    let startY = 0
    let longPressed = false
    const cancelPress = () => clearTimeout(timer)
    card.addEventListener('pointerdown', event => {
      if (state.selecting || event.button !== 0) return
      startX = event.clientX
      startY = event.clientY
      longPressed = false
      timer = setTimeout(() => {
        longPressed = true
        state.selecting = true
        state.selectedEvents.add(card.dataset.eventId)
        if ('vibrate' in navigator) navigator.vibrate(35)
        renderEvents(state.data?.events || [])
      }, 520)
    })
    card.addEventListener('pointermove', event => {
      if (Math.hypot(event.clientX - startX, event.clientY - startY) > 8) cancelPress()
    })
    card.addEventListener('pointerup', cancelPress)
    card.addEventListener('pointercancel', cancelPress)
    card.addEventListener('pointerleave', cancelPress)
    card.addEventListener('click', event => {
      if (longPressed) {
        event.preventDefault()
        return
      }
      if (state.selecting && !event.target.closest('input')) toggleEventSelection(card.dataset.eventId)
    })
  })
}

async function deleteSelectedEvents() {
  const ids = [...state.selectedEvents]
  if (!ids.length) return
  if (!window.confirm(`确定删除选中的 ${ids.length} 条任务记录吗？此操作无法撤销。`)) return
  const button = $('deleteSelectedButton')
  button.disabled = true
  button.textContent = '正在删除'
  try {
    const result = await api('/api/events/delete', { method: 'POST', body: { ids } })
    state.data.events = state.data.events.filter(event => !state.selectedEvents.has(event.id))
    state.selectedEvents.clear()
    state.selecting = false
    renderStats(state.data.events)
    renderEvents(state.data.events)
    toast(`已删除 ${result.removed} 条任务记录`)
  } catch (error) {
    toast(`删除失败：${error.message}`)
    updateSelectionUi()
  } finally {
    button.textContent = '删除'
  }
}

function renderDevices(devices, pending, discovered) {
  $('pendingList').innerHTML = pending.map(p => `<div class="pending-card"><div><strong>${esc(p.name)}</strong><div class="card-copy"><span>${esc(p.ip)} · 配对码 ${esc(p.code)}</span></div></div><div class="pending-actions"><button class="secondary-button" data-approve="${p.id}">允许</button><button class="tiny-button" data-deny="${p.id}">拒绝</button></div></div>`).join('')
  $('deviceList').innerHTML = devices.length ? devices.map(d => `<div class="device-card"><div class="device-avatar">${esc(d.name.slice(0,1).toUpperCase())}</div><div class="card-copy"><strong>${esc(d.name)}</strong><span>${esc(d.last_ip || '局域网设备')}</span></div><button class="tiny-button" data-revoke="${d.id}">撤销</button></div>`).join('') : '<div class="empty-state"><i>+</i><p>还没有已授权手机</p></div>'
  $('discoveredList').innerHTML = discovered.length ? `<p class="eyebrow">附近设备</p>${discovered.map(d => `<div class="device-card"><div class="device-avatar">${esc((d.name || '?').slice(0,1))}</div><div class="card-copy"><strong>${esc(d.name)}</strong><span>${esc(d.model || d.role)} · ${esc(d.ip)}</span></div></div>`).join('')}` : ''
  document.querySelectorAll('[data-approve]').forEach(b => b.onclick = () => approve(b.dataset.approve, true))
  document.querySelectorAll('[data-deny]').forEach(b => b.onclick = () => approve(b.dataset.deny, false))
  document.querySelectorAll('[data-revoke]').forEach(b => b.onclick = () => revoke(b.dataset.revoke))
}

function renderAdapters(adapters) {
  const short = { codex: 'CX', 'deepseek-harness-eac': 'DS', 'claude-code-haha': 'HH', opencode: 'OC', openclaw: 'CL', 'hermes-agent': 'HE' }
  $('adapterList').innerHTML = adapters.map(a => `<div class="adapter-card"><div class="agent-avatar">${short[a.id] || 'AI'}</div><div class="card-copy"><strong>${esc(a.name)}</strong><span>${esc(a.mode)}</span></div><div><span class="state-badge">默认监听</span></div></div>`).join('')
}

async function approve(deviceId, allow) {
  await api('/api/pair/approve', { method: 'POST', body: { device_id: deviceId, allow } })
  toast(allow ? '设备已授权' : '已拒绝设备')
  await load()
}

async function revoke(deviceId) {
  await api('/api/device/revoke', { method: 'POST', body: { device_id: deviceId, allow: false } })
  toast('设备授权已撤销')
  await load()
}

function connectWebSocket() {
  if (state.ws && [WebSocket.OPEN, WebSocket.CONNECTING].includes(state.ws.readyState)) return
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
  const ws = new WebSocket(`${protocol}//${location.host}/ws?token=${encodeURIComponent(state.token)}`)
  state.ws = ws
  ws.onopen = () => {
    state.reconnect = 0
    document.querySelector('.connection').classList.add('live')
    $('liveText').textContent = '实时连接'
  }
  ws.onmessage = event => {
    const item = JSON.parse(event.data)
    state.data.events.unshift(item)
    state.data.events = state.data.events.slice(0, 100)
    renderStats(state.data.events)
    renderEvents(state.data.events)
    notify(item)
  }
  ws.onclose = () => {
    document.querySelector('.connection').classList.remove('live')
    $('liveText').textContent = '正在重连'
    const delay = Math.min(1000 * 2 ** state.reconnect++, 15000)
    setTimeout(connectWebSocket, delay)
  }
}

async function requestNotifications(showToast = true) {
  if (!('Notification' in window)) {
    if (showToast) toast('当前浏览器不支持系统通知，页面保持打开时仍会震动和响铃')
    return
  }
  try {
    const result = await Notification.requestPermission()
    if (showToast) toast(result === 'granted' ? '手机通知已开启' : '未获得系统通知权限')
  } catch {
    if (showToast) toast('HTTP 局域网页面只能使用前台响铃；系统通知需要 HTTPS 或通知应用')
  }
  updateNotifyButton()
}

function updateNotifyButton() {
  const granted = 'Notification' in window && Notification.permission === 'granted'
  $('notifyButton').textContent = granted ? '手机通知已开启' : '开启手机通知'
  $('notifyButton').disabled = granted
}

function notify(item) {
  if ('vibrate' in navigator) navigator.vibrate([120, 70, 180])
  playTone(item.kind === 'failed' ? 260 : 540)
  if ('Notification' in window && Notification.permission === 'granted') {
    new Notification(`${item.agent} · ${item.title}`, { body: item.message, tag: item.conversation_id || item.id, renotify: true })
  }
  toast(`${item.agent}：${item.title}`)
}

function playTone(frequency) {
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)()
    const osc = ctx.createOscillator()
    const gain = ctx.createGain()
    osc.frequency.value = frequency
    gain.gain.setValueAtTime(.08, ctx.currentTime)
    gain.gain.exponentialRampToValueAtTime(.001, ctx.currentTime + .42)
    osc.connect(gain).connect(ctx.destination)
    osc.start(); osc.stop(ctx.currentTime + .42)
  } catch {}
}

document.querySelectorAll('.nav-item').forEach(button => button.addEventListener('click', () => {
  const tab = button.dataset.tab
  document.querySelectorAll('.nav-item').forEach(b => b.classList.toggle('active', b === button))
  document.querySelectorAll('.tab-page').forEach(p => p.classList.add('hidden'))
  $(`${tab}Tab`).classList.remove('hidden')
  const labels = { overview: ['通知中心', '最近任务'], devices: ['局域网连接', '设备与授权'], agents: ['事件来源', 'Agent 接入'], diagnostics: ['排查问题', '运行诊断'] }
  $('pageEyebrow').textContent = labels[tab][0]
  $('pageTitle').textContent = labels[tab][1]
}))

async function loadDiagnostics() {
  try { const data = await api('/api/diagnostics'); $('logPath').textContent = data.log_path; $('logTail').textContent = data.log_tail || '日志暂为空' }
  catch (error) { $('logTail').textContent = error.message }
}
$('refreshDiagnostics').addEventListener('click', loadDiagnostics)
document.querySelector('[data-tab="diagnostics"]').addEventListener('click', loadDiagnostics)

$('pairButton').addEventListener('click', pairDevice)
$('notifyButton').addEventListener('click', () => requestNotifications(true))
$('testButton').addEventListener('click', async () => { await api('/api/test', { method: 'POST' }); toast('测试通知已发送') })
$('selectButton').addEventListener('click', () => setSelectionMode(!state.selecting))
$('selectAllButton').addEventListener('click', () => {
  const events = state.data?.events || []
  if (state.selectedEvents.size === events.length) state.selectedEvents.clear()
  else events.forEach(event => state.selectedEvents.add(event.id))
  renderEvents(events)
})
$('deleteSelectedButton').addEventListener('click', deleteSelectedEvents)
load()
