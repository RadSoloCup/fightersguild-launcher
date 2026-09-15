import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { openUrl } from '@tauri-apps/plugin-opener'

const appWindow = getCurrentWindow()

const statusDot = document.getElementById('status-dot')
const statusText = document.getElementById('status-text')
const progressTrack = document.getElementById('progress-track')
const progressFill = document.getElementById('progress-fill')
const log = document.getElementById('log')
const btnPrimary = document.getElementById('btn-primary')
const btnRetry = document.getElementById('btn-retry')
const updateBanner = document.getElementById('update-banner')
const updateText = document.getElementById('update-text')
const btnUpdate = document.getElementById('btn-update')
const versionText = document.getElementById('version-text')
const linkMinecraft = document.getElementById('link-minecraft')
const btnExit = document.getElementById('btn-exit')

let pendingUpdate = null
// What btn-primary should do right now: 'install' | 'update' | 'play'
let primaryAction = 'install'

function setStatus(text, kind) {
  statusText.textContent = text
  statusDot.className = 'status-dot' + (kind ? ' ' + kind : '')
}

function setProgress(pct) {
  progressFill.style.width = Math.max(0, Math.min(100, pct)) + '%'
}

function appendLog(text, isError) {
  const line = document.createElement('div')
  if (isError) line.className = 'error'
  line.textContent = text
  log.appendChild(line)
  log.scrollTop = log.scrollHeight
}

function showPrimaryButton(action) {
  primaryAction = action
  btnPrimary.hidden = false
  btnPrimary.textContent = action === 'install' ? 'Install' : action === 'update' ? 'Update' : 'Play'
}

// ── window chrome ───────────────────────────────────────────────────────
document.getElementById('tb-minimize').addEventListener('click', () => appWindow.minimize())
document.getElementById('tb-close').addEventListener('click', () => appWindow.close())
const tbMaximize = document.getElementById('tb-maximize')
const MAXIMIZE_ICON = '\uE922'
const RESTORE_ICON = '\uE923'
async function syncMaximizeIcon() {
  tbMaximize.textContent = (await appWindow.isMaximized()) ? RESTORE_ICON : MAXIMIZE_ICON
}
tbMaximize.addEventListener('click', async () => {
  await appWindow.toggleMaximize()
  syncMaximizeIcon()
})
syncMaximizeIcon()
btnExit.addEventListener('click', () => appWindow.close())

linkMinecraft.addEventListener('click', e => {
  e.preventDefault()
  openUrl('https://fightersguild.playit.quest/minecraft')
})

// ── news card ────────────────────────────────────────────────────────────
async function loadNews() {
  try {
    const res = await fetch('https://fightersguild.playit.quest/minecraft/updates-index.json')
    const posts = await res.json()
    if (!posts.length) return
    const latest = posts[0]
    document.getElementById('news-title').textContent = latest.title
    document.getElementById('news-excerpt').textContent = latest.excerpt
    const link = document.getElementById('news-link')
    link.addEventListener('click', e => {
      e.preventDefault()
      openUrl('https://fightersguild.playit.quest' + latest.url)
    })
    const img = document.getElementById('news-image')
    img.src = 'https://fightersguild.playit.quest/updates/assets/pixelmon-gallery.webp'
    img.alt = latest.title
    if (/battlepass|trick or trainer/i.test(latest.title + latest.excerpt)) {
      document.getElementById('news-battlepass').hidden = false
    }
    document.getElementById('news-card').hidden = false
  } catch (err) {
    console.error('news load failed', err)
  }
}

// ── install / update / play flow ────────────────────────────────────────
async function checkStatus() {
  setStatus('Checking your Minecraft install...')
  try {
    const status = await invoke('check_status')
    if (!status.vanillaInstalled) {
      setStatus('Vanilla Minecraft 1.20.1 not found', 'error')
      appendLog('Open the official Minecraft Launcher, select version 1.20.1, and click Play once, then reopen this.', true)
      return
    }
    if (status.action === 'ready') {
      setStatus('Ready to play', 'ok')
      setProgress(100)
      showPrimaryButton('play')
      return
    }
    if (status.action === 'update') {
      setStatus(`Update available (installed v${status.installedVersion}, latest v${status.latestVersion})`)
      showPrimaryButton('update')
      return
    }
    setStatus('Ready to install the modpack')
    showPrimaryButton('install')
  } catch (err) {
    setStatus('Could not check for updates', 'error')
    appendLog(String(err), true)
    btnRetry.hidden = false
  }
}

async function runInstall() {
  btnPrimary.hidden = true
  btnRetry.hidden = true
  progressTrack.hidden = false
  log.hidden = false
  setProgress(0)
  log.innerHTML = ''
  setStatus('Starting...')

  try {
    await invoke('run_install')
  } catch (err) {
    setStatus('Something went wrong', 'error')
    appendLog(String(err), true)
    btnRetry.hidden = false
  }
}

listen('install-progress', event => {
  const { message, percent } = event.payload
  setStatus(message)
  appendLog(message)
  if (typeof percent === 'number') setProgress(percent)
})

listen('install-done', () => {
  setStatus('Ready to play', 'ok')
  setProgress(100)
  showPrimaryButton('play')
})

listen('install-error', event => {
  setStatus('Something went wrong', 'error')
  appendLog(event.payload, true)
  btnRetry.hidden = false
})

btnPrimary.addEventListener('click', async () => {
  if (primaryAction === 'play') {
    btnPrimary.disabled = true
    try {
      await invoke('launch_minecraft')
    } catch (err) {
      appendLog(String(err), true)
    }
    btnPrimary.disabled = false
    return
  }
  // install or update — same flow, just different starting label
  runInstall()
})

btnRetry.addEventListener('click', () => {
  btnRetry.hidden = true
  checkStatus()
})

btnUpdate.addEventListener('click', async () => {
  if (!pendingUpdate) return
  btnUpdate.disabled = true
  btnUpdate.textContent = 'Updating...'
  try {
    await pendingUpdate.downloadAndInstall()
    await relaunch()
  } catch (err) {
    appendLog('Update failed: ' + err, true)
    btnUpdate.disabled = false
    btnUpdate.textContent = 'Update & Restart'
  }
})

async function checkForLauncherUpdate() {
  try {
    const update = await check()
    if (update?.available) {
      pendingUpdate = update
      updateText.textContent = `A launcher update is available (v${update.version}).`
      updateBanner.hidden = false
    }
  } catch (err) {
    console.error('update check failed', err)
  }
}

async function init() {
  versionText.textContent = 'v' + await getVersion()
  loadNews()
  checkForLauncherUpdate()
  checkStatus()
}

init()
