import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ChangeEvent, DragEvent, ReactNode } from 'react';
import {
  ArrowLeft,
  Check,
  CheckCircle2,
  ChevronRight,
  ClipboardPaste,
  Copy,
  CircleAlert,
  Clock3,
  File,
  FileText,
  FolderOpen,
  FolderPlus,
  HardDrive,
  Inbox,
  Languages,
  Laptop,
  Link2,
  Monitor,
  Moon,
  QrCode,
  RefreshCw,
  Send as SendIcon,
  Server,
  Settings as SettingsIcon,
  Shield,
  ShieldCheck,
  Smartphone,
  Sun,
  UploadCloud,
  Wifi,
  X,
} from 'lucide-react';
import {
  createStorageDirectory,
  decideLinkShareRequest,
  decidePending,
  getDevices,
  getEnvironmentSettings,
  getHistory,
  getIncomingTransfers,
  getLinkShare,
  getNetworkSettings,
  getPending,
  getStatus,
  getStorageSettings,
  getTransfers,
  probeDevice,
  scanDevices,
  sendFiles,
  sendText,
  startLinkShare,
  stopLinkShare,
  listStorageDirectories,
  updateStorageSettings,
  updateNetworkSettings,
  updateEnvironmentSettings,
  updateLinkShare,
} from './api';
import type { UploadProgress } from './api';
import { ConfirmDialog } from './components/ConfirmDialog';
import { LinkShareView } from './components/LinkShareView';
import { detectLocale, messages } from './i18n';
import { formatBytes, formatTime } from './format';
import type {
  DeviceInfo,
  AliasLocale,
  EnvironmentSettings,
  IncomingTransfer,
  LinkShare,
  Locale,
  NetworkInterfaceInfo,
  NetworkMode,
  NetworkSettings,
  OutgoingTransfer,
  PendingTransfer,
  ReceivedFile,
  StatusResponse,
  StorageSettings,
  DirectoryListing,
  Tab,
} from './types';

type Theme = 'system' | 'light' | 'dark';
type SendMode = 'files' | 'text';
type IconComponent = typeof SendIcon;
const MAX_CLIPBOARD_BYTES = 1024 * 1024;

const navItems: Array<{ id: Tab; icon: IconComponent; label: string }> = [
  { id: 'send', icon: SendIcon, label: 'send' },
  { id: 'receive', icon: Inbox, label: 'receive' },
  { id: 'settings', icon: SettingsIcon, label: 'settings' },
];

const deviceKey = (device: DeviceInfo) => `${device.fingerprint}:${device.ip ?? ''}:${device.port}`;

export default function App() {
  const [locale, setLocale] = useState<Locale>(detectLocale);
  const [theme, setTheme] = useState<Theme>(() => (localStorage.getItem('localsendy-theme') as Theme) || 'system');
  const [activeTab, setActiveTab] = useState<Tab>('send');
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [pending, setPending] = useState<PendingTransfer | null>(null);
  const [history, setHistory] = useState<ReceivedFile[]>([]);
  const [transfers, setTransfers] = useState<OutgoingTransfer[]>([]);
  const [incomingTransfers, setIncomingTransfers] = useState<IncomingTransfer[]>([]);
  const [selectedDeviceIds, setSelectedDeviceIds] = useState<Set<string>>(new Set());
  const [files, setFiles] = useState<File[]>([]);
  const [sendMode, setSendMode] = useState<SendMode>('files');
  const [textMessage, setTextMessage] = useState('');
  const [isScanning, setIsScanning] = useState(false);
  const [manualAddress, setManualAddress] = useState('');
  const [isProbing, setIsProbing] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [browserUploadProgress, setBrowserUploadProgress] = useState<UploadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [linkShare, setLinkShare] = useState<LinkShare | null>(null);
  const [isLinkShareView, setIsLinkShareView] = useState(false);
  const [isStartingLinkShare, setIsStartingLinkShare] = useState(false);
  const [linkShareUploadProgress, setLinkShareUploadProgress] = useState<UploadProgress | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const copy = useMemo(() => messages(locale), [locale]);

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextDevices, nextPending, nextHistory, nextTransfers, nextIncomingTransfers] = await Promise.all([
        getStatus(),
        getDevices(),
        getPending(),
        getHistory(),
        getTransfers(),
        getIncomingTransfers(),
      ]);
      setStatus(nextStatus);
      setDevices(nextDevices);
      setPending(nextPending);
      setHistory(nextHistory);
      setTransfers(nextTransfers);
      setIncomingTransfers(nextIncomingTransfers);
      setSelectedDeviceIds((current) => new Set(
        nextDevices.filter((device) => current.has(deviceKey(device))).map(deviceKey),
      ));
      setError(null);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.error);
    }
  }, [copy.error]);

  const hasActiveTransfers = transfers.some((transfer) => transfer.status === 'preparing' || transfer.status === 'sending')
    || incomingTransfers.some((transfer) => transfer.status === 'waiting' || transfer.status === 'receiving');

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    const interval = isSending || hasActiveTransfers ? 500 : 2500;
    const poll = async () => {
      await refresh();
      if (!cancelled) timer = window.setTimeout(poll, interval);
    };
    void poll();
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [hasActiveTransfers, isSending, refresh]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem('localsendy-theme', theme);
  }, [theme]);

  useEffect(() => {
    document.documentElement.lang = locale;
    localStorage.setItem('localsendy-locale', locale);
  }, [locale]);

  useEffect(() => {
    if (!notice) return undefined;
    const timer = window.setTimeout(() => setNotice(null), 5000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  async function handleScan() {
    setIsScanning(true);
    try {
      await scanDevices();
      await new Promise((resolve) => window.setTimeout(resolve, 1400));
      await refresh();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsScanning(false);
    }
  }

  async function handleProbe() {
    if (!manualAddress.trim()) return;
    setIsProbing(true);
    setError(null);
    try {
      const device = await probeDevice(manualAddress.trim());
      setSelectedDeviceIds((current) => new Set(current).add(deviceKey(device)));
      setManualAddress('');
      setNotice(`${copy.deviceAdded}: ${device.alias}`);
      await refresh();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsProbing(false);
    }
  }

  function addFiles(nextFiles: File[]) {
    setFiles((current) => {
      const existing = new Set(current.map((file) => `${file.name}:${file.size}:${file.lastModified}`));
      return [...current, ...nextFiles.filter((file) => !existing.has(`${file.name}:${file.size}:${file.lastModified}`))];
    });
  }

  function handleFileInput(event: ChangeEvent<HTMLInputElement>) {
    addFiles(Array.from(event.target.files ?? []));
    event.target.value = '';
  }

  function handleDrop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setIsDragging(false);
    addFiles(Array.from(event.dataTransfer.files));
  }

  async function handleSend() {
    const targets = devices.filter((device) => selectedDeviceIds.has(deviceKey(device)));
    if (targets.length === 0) return;
    if (sendMode === 'files' && files.length === 0) return;
    if (sendMode === 'text' && !textMessage.trim()) return;
    setIsSending(true);
    setBrowserUploadProgress(sendMode === 'files' ? {
      loaded: 0,
      total: files.reduce((total, file) => total + file.size, 0),
    } : null);
    setError(null);
    setNotice(null);
    try {
      const result = sendMode === 'files'
        ? await sendFiles(targets, files, undefined, setBrowserUploadProgress)
        : await sendText(targets, textMessage);
      const succeeded = result.transfers.filter((transfer) => transfer.success);
      const failed = result.transfers.filter((transfer) => !transfer.success);
      await refresh();
      if (succeeded.length > 0) {
        if (sendMode === 'files') setFiles([]);
        else setTextMessage('');
        setNotice(`${copy.transferComplete}: ${succeeded.length} / ${targets.length}`);
      }
      if (failed.length > 0) {
        setError(failed.map((transfer) => `${transfer.targetAlias}: ${transfer.error ?? copy.transferFailed}`).join('\n'));
      }
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.transferFailed);
    } finally {
      setIsSending(false);
      setBrowserUploadProgress(null);
    }
  }

  async function handlePending(decision: 'accept' | 'reject') {
    try {
      await decidePending(decision);
      await refresh();
      setNotice(decision === 'accept' ? copy.accept : copy.reject);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.error);
    }
  }

  async function handleStartLinkShare() {
    if (files.length === 0) return;
    setIsStartingLinkShare(true);
    setError(null);
    setLinkShareUploadProgress({ loaded: 0, total: files.reduce((total, file) => total + file.size, 0) });
    try {
      const next = await startLinkShare(files, false, '', setLinkShareUploadProgress);
      setLinkShare(next);
      setIsLinkShareView(true);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsStartingLinkShare(false);
      setLinkShareUploadProgress(null);
    }
  }

  async function handleStopLinkShare() {
    const shareId = linkShare?.shareId;
    if (!shareId) {
      setLinkShare(null);
      setIsLinkShareView(false);
      return;
    }
    try {
      await stopLinkShare(shareId);
      setLinkShare(null);
      setIsLinkShareView(false);
      setNotice(copy.linkShareStopped);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.error);
    }
  }

  useEffect(() => {
    if (!isLinkShareView) return undefined;
    let cancelled = false;
    const poll = async () => {
      try {
        const next = await getLinkShare();
        if (!cancelled) {
          setLinkShare(next.active ? next : null);
          if (!next.active) setIsLinkShareView(false);
        }
      } catch (requestError) {
        if (!cancelled) setError(requestError instanceof Error ? requestError.message : copy.error);
      }
    };
    void poll();
    const timer = window.setInterval(poll, 1000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [copy.error, isLinkShareView]);

  useEffect(() => {
    const shareId = linkShare?.shareId;
    if (!isLinkShareView || !shareId) return undefined;
    const stopOnExit = () => { void stopLinkShare(shareId, true).catch(() => undefined); };
    window.addEventListener('pagehide', stopOnExit);
    return () => window.removeEventListener('pagehide', stopOnExit);
  }, [isLinkShareView, linkShare?.shareId]);

  const nav = (id: Tab) => {
    if (isLinkShareView && id !== 'send') void handleStopLinkShare();
    setActiveTab(id);
  };

  function toggleDevice(device: DeviceInfo) {
    const key = deviceKey(device);
    setSelectedDeviceIds((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">{copy.skipContent}</a>
      <aside className="sidebar" aria-label="Primary navigation">
        <Brand copy={copy} />
        <nav className="nav-list">
          {navItems.map(({ id, icon: Icon, label }) => (
            <button
              key={id}
              className={`nav-item ${activeTab === id ? 'active' : ''}`}
              type="button"
              aria-current={activeTab === id ? 'page' : undefined}
              onClick={() => nav(id)}
            >
              <Icon size={19} strokeWidth={1.8} aria-hidden="true" />
              <span>{copy[label]}</span>
              {id === 'receive' && pending ? <span className="nav-badge">1</span> : null}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          <StatusPill status={status} copy={copy} />
          <span className="version-label">v{status?.version ?? '0.3.0'}</span>
        </div>
      </aside>

      <main id="main-content" className="main-content" tabIndex={-1}>
        <header className="topbar">
          <div className="topbar-status">
            <img className="topbar-logo" src="/localsendy-192.png" alt="" aria-hidden="true" />
            <span className="status-dot" aria-hidden="true" />
            <span>{status?.alias ?? copy.brand}</span>
            <span className="topbar-divider" aria-hidden="true" />
            <span className="muted-text">{status ? `${status.nearbyDevices} ${copy.nearbyDevices.toLowerCase()}` : copy.scanning}</span>
          </div>
          <div className="topbar-actions">
            <button className="icon-button" type="button" title={copy.refresh} aria-label={copy.refresh} onClick={refresh}>
              <RefreshCw size={18} aria-hidden="true" />
            </button>
            <label className="language-select compact-select">
              <Languages size={16} aria-hidden="true" />
              <span className="sr-only">{copy.language}</span>
              <select value={locale} onChange={(event) => setLocale(event.target.value as Locale)}>
                <option value="en">EN</option>
                <option value="zh-CN">简中</option>
                <option value="zh-TW">繁中</option>
              </select>
            </label>
          </div>
        </header>

        {error ? (
          <div className="alert error-alert" role="alert">
            <CircleAlert size={18} aria-hidden="true" />
            <span>{error}</span>
            <button className="alert-close" type="button" aria-label={copy.clear} onClick={() => setError(null)}><X size={16} /></button>
          </div>
        ) : null}
        {notice ? (
          <div className="alert success-alert" role="status" aria-live="polite">
            <Check size={18} aria-hidden="true" />
            <span>{notice}</span>
            <button className="alert-close" type="button" aria-label={copy.clear} onClick={() => setNotice(null)}><X size={16} /></button>
          </div>
        ) : null}

        {activeTab === 'send' && isLinkShareView && linkShare ? <LinkShareView copy={copy} share={linkShare} onShare={setLinkShare} onBack={handleStopLinkShare} onError={setError} onNotice={setNotice} /> : null}
        {activeTab === 'send' && !isLinkShareView ? (
          <SendView
            copy={copy}
            devices={devices}
            selectedDeviceIds={selectedDeviceIds}
            files={files}
            transfers={transfers}
            sendMode={sendMode}
            textMessage={textMessage}
            isScanning={isScanning}
            manualAddress={manualAddress}
            isProbing={isProbing}
            isSending={isSending}
            browserUploadProgress={browserUploadProgress}
            isDragging={isDragging}
            fileInputRef={fileInputRef}
            onScan={handleScan}
            onManualAddress={setManualAddress}
            onProbe={handleProbe}
            onSelectDevice={toggleDevice}
            onSendMode={setSendMode}
            onTextMessage={setTextMessage}
            onFileInput={handleFileInput}
            onDrop={handleDrop}
            onDragState={setIsDragging}
            onClearFiles={() => setFiles([])}
            onRemoveFile={(index) => setFiles((current) => current.filter((_, currentIndex) => currentIndex !== index))}
            onSend={handleSend}
            onLinkShare={handleStartLinkShare}
            isStartingLinkShare={isStartingLinkShare}
            linkShareUploadProgress={linkShareUploadProgress}
          />
        ) : null}
        {activeTab === 'receive' ? <ReceiveView copy={copy} status={status} pending={pending} history={history} incomingTransfers={incomingTransfers} onDecision={handlePending} /> : null}
        {activeTab === 'settings' ? <SettingsView copy={copy} locale={locale} theme={theme} status={status} onLocale={setLocale} onTheme={setTheme} onError={setError} onNotice={setNotice} /> : null}
      </main>

      <nav className="mobile-nav" aria-label="Primary navigation">
        {navItems.map(({ id, icon: Icon, label }) => (
          <button key={id} type="button" className={`mobile-nav-item ${activeTab === id ? 'active' : ''}`} onClick={() => nav(id)} aria-current={activeTab === id ? 'page' : undefined}>
            <Icon size={20} aria-hidden="true" />
            <span>{copy[label]}</span>
            {id === 'receive' && pending ? <span className="mobile-badge" /> : null}
          </button>
        ))}
      </nav>
    </div>
  );
}

function Brand({ copy }: { copy: Record<string, string> }) {
  return (
    <div className="brand-lockup">
      <img className="brand-logo" src="/localsendy-192.png" alt="" aria-hidden="true" />
      <div><strong>{copy.brand}</strong><span>{copy.tagline}</span></div>
    </div>
  );
}

function StatusPill({ status, copy }: { status: StatusResponse | null; copy: Record<string, string> }) {
  return <div className="status-pill"><span className="status-dot" aria-hidden="true" /><span>{status ? copy.online : copy.scanning}</span></div>;
}

function SendView(props: {
  copy: Record<string, string>;
  devices: DeviceInfo[];
  selectedDeviceIds: Set<string>;
  files: File[];
  transfers: OutgoingTransfer[];
  sendMode: SendMode;
  textMessage: string;
  isScanning: boolean;
  manualAddress: string;
  isProbing: boolean;
  isSending: boolean;
  browserUploadProgress: UploadProgress | null;
  isDragging: boolean;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  onScan: () => void;
  onManualAddress: (address: string) => void;
  onProbe: () => void;
  onSelectDevice: (device: DeviceInfo) => void;
  onSendMode: (mode: SendMode) => void;
  onTextMessage: (text: string) => void;
  onFileInput: (event: ChangeEvent<HTMLInputElement>) => void;
  onDrop: (event: DragEvent<HTMLDivElement>) => void;
  onDragState: (state: boolean) => void;
  onClearFiles: () => void;
  onRemoveFile: (index: number) => void;
  onSend: () => void;
  onLinkShare: () => void;
  isStartingLinkShare: boolean;
  linkShareUploadProgress: UploadProgress | null;
}) {
  const { copy } = props;
  const [clipboardError, setClipboardError] = useState<string | null>(null);
  const selectedCount = props.selectedDeviceIds.size;
  const textBytes = new TextEncoder().encode(props.textMessage).length;
  const textTooLarge = textBytes > MAX_CLIPBOARD_BYTES;
  const payloadReady = props.sendMode === 'files' ? props.files.length > 0 : props.textMessage.trim().length > 0 && !textTooLarge;

  async function pasteClipboard() {
    setClipboardError(null);
    try {
      props.onTextMessage(await navigator.clipboard.readText());
    } catch {
      setClipboardError(copy.clipboardUnavailable);
    }
  }

  return (
    <section className="workspace send-workspace">
      <div className="page-intro">
        <div>
          <p className="eyebrow"><Wifi size={14} aria-hidden="true" /> {copy.localNetwork.toUpperCase()}</p>
          <h1>{copy.sendHeadline}</h1>
          <p className="page-subhead">{copy.sendSubhead}</p>
        </div>
        <div className="protocol-note"><ShieldCheck size={17} aria-hidden="true" /><span>HTTPS / LocalSend v2</span></div>
      </div>

      <div className="send-mode-control" role="tablist" aria-label={copy.sendContentType}>
        <button type="button" role="tab" aria-selected={props.sendMode === 'files'} className={props.sendMode === 'files' ? 'active' : ''} onClick={() => props.onSendMode('files')}><File size={17} aria-hidden="true" />{copy.files}</button>
        <button type="button" role="tab" aria-selected={props.sendMode === 'text'} className={props.sendMode === 'text' ? 'active' : ''} onClick={() => props.onSendMode('text')}><FileText size={17} aria-hidden="true" />{copy.clipboardText}</button>
      </div>

      <div className="link-share-entry">
        <div><span className="link-share-entry-icon"><Link2 size={18} aria-hidden="true" /></span><span><strong>{copy.shareViaLink}</strong><small>{copy.shareViaLinkHint}</small></span></div>
        <button className="secondary-button" type="button" disabled={props.files.length === 0 || props.sendMode !== 'files' || props.isStartingLinkShare} onClick={props.onLinkShare}>
          {props.isStartingLinkShare ? <RefreshCw size={17} className="spin" aria-hidden="true" /> : <Link2 size={17} aria-hidden="true" />}
          {props.isStartingLinkShare ? copy.startingLinkShare : copy.shareViaLink}
        </button>
        {props.linkShareUploadProgress ? <ProgressBar value={props.linkShareUploadProgress.loaded} total={props.linkShareUploadProgress.total} label={`${copy.uploadingToServer} ${formatBytes(props.linkShareUploadProgress.loaded)} / ${formatBytes(props.linkShareUploadProgress.total)}`} /> : null}
      </div>

      <div className="send-grid">
        <div className="send-column">
          {props.sendMode === 'files' ? <>
            <div className="section-heading"><div><h2>{copy.chooseFiles}</h2><p>{copy.fileHint}</p></div>{props.files.length > 0 ? <button className="text-button" type="button" onClick={props.onClearFiles}>{copy.clear}</button> : null}</div>
            <div
              className={`dropzone ${props.isDragging ? 'dragging' : ''}`}
              onDragEnter={(event) => { event.preventDefault(); props.onDragState(true); }}
              onDragOver={(event) => event.preventDefault()}
              onDragLeave={(event) => { if (event.currentTarget === event.target) props.onDragState(false); }}
              onDrop={props.onDrop}
            >
              <div className="dropzone-icon"><UploadCloud size={25} aria-hidden="true" /></div>
              <strong>{copy.dropFiles}</strong>
              <span>{copy.fileHint}</span>
              <button className="secondary-button" type="button" onClick={() => props.fileInputRef.current?.click()}><FolderOpen size={17} aria-hidden="true" />{copy.chooseFiles}</button>
              <input ref={props.fileInputRef} className="sr-only" type="file" multiple aria-label={copy.chooseFiles} onChange={props.onFileInput} />
            </div>
            {props.files.length > 0 ? <div className="file-list-section"><div className="section-heading compact"><h2>{copy.selectedFiles}<span className="count-badge">{props.files.length}</span></h2></div><div className="file-list">{props.files.map((file, index) => <FileRow key={`${file.name}-${file.lastModified}`} copy={copy} file={file} onRemove={() => props.onRemoveFile(index)} />)}</div></div> : null}
          </> : <div className="text-composer">
            <div className="section-heading"><div><h2>{copy.clipboardText}</h2><p>{copy.textHint}</p></div></div>
            <label htmlFor="text-message">{copy.message}</label>
            <textarea id="text-message" value={props.textMessage} onChange={(event) => props.onTextMessage(event.target.value)} placeholder={copy.textPlaceholder} rows={9} />
            <div className="text-composer-footer">
              <span>{props.textMessage.length} {copy.characters} · {formatBytes(textBytes)}</span>
              <div>
                <button className="secondary-button" type="button" onClick={pasteClipboard}><ClipboardPaste size={17} aria-hidden="true" />{copy.pasteClipboard}</button>
                <button className="text-button" type="button" disabled={!props.textMessage} onClick={() => props.onTextMessage('')}>{copy.clear}</button>
              </div>
            </div>
            {clipboardError ? <p className="field-error" role="alert">{clipboardError}</p> : null}
            {textTooLarge ? <p className="field-error" role="alert">{copy.textTooLarge} ({formatBytes(MAX_CLIPBOARD_BYTES)})</p> : null}
          </div>}
        </div>

        <div className="devices-column">
          <div className="section-heading device-heading"><div><h2>{copy.nearbyDevices}</h2><p>{props.devices.length === 0 ? copy.noDevicesHint : `${selectedCount} ${copy.devicesSelected} · ${props.devices.length} ${copy.online.toLowerCase()}`}</p></div><button className="icon-button outlined" type="button" title={props.isScanning ? copy.scanning : copy.scan} aria-label={props.isScanning ? copy.scanning : copy.scan} onClick={props.onScan} disabled={props.isScanning}><RefreshCw size={17} className={props.isScanning ? 'spin' : ''} aria-hidden="true" /></button></div>
          {props.devices.length > 0 ? <div className="device-list">{props.devices.map((device) => <DeviceCard key={deviceKey(device)} device={device} selected={props.selectedDeviceIds.has(deviceKey(device))} copy={copy} onSelect={() => props.onSelectDevice(device)} />)}</div> : <div className="empty-device-state"><div className="empty-icon"><Wifi size={22} aria-hidden="true" /></div><strong>{copy.noDevices}</strong><span>{copy.noDevicesHint}</span><button className="secondary-button" type="button" onClick={props.onScan} disabled={props.isScanning}><RefreshCw size={17} className={props.isScanning ? 'spin' : ''} aria-hidden="true" />{props.isScanning ? copy.scanning : copy.scan}</button></div>}
          <div className="manual-target">
            <label htmlFor="manual-address">{copy.manualAddress}</label>
            <p>{copy.manualHint}</p>
            <div className="manual-target-row">
              <input id="manual-address" type="text" inputMode="url" autoComplete="off" placeholder="192.168.1.50[:53317]" value={props.manualAddress} onChange={(event) => props.onManualAddress(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') props.onProbe(); }} />
              <button className="secondary-button" type="button" disabled={props.isProbing || !props.manualAddress.trim()} onClick={props.onProbe}>{props.isProbing ? <RefreshCw size={17} className="spin" aria-hidden="true" /> : <Wifi size={17} aria-hidden="true" />}{props.isProbing ? copy.connecting : copy.connect}</button>
            </div>
          </div>
          <button className="primary-button send-cta" type="button" disabled={selectedCount === 0 || !payloadReady || props.isSending} onClick={props.onSend}>{props.isSending ? <RefreshCw size={18} className="spin" aria-hidden="true" /> : <SendIcon size={18} aria-hidden="true" />}{props.isSending ? copy.sending : selectedCount > 0 ? `${copy.sendToDevices} (${selectedCount})` : copy.selectDevice}<ChevronRight size={17} aria-hidden="true" /></button>
          {props.browserUploadProgress ? <div className="browser-upload-progress"><ProgressBar
            value={props.browserUploadProgress.loaded}
            total={props.browserUploadProgress.total}
            label={props.browserUploadProgress.total > 0 && props.browserUploadProgress.loaded >= props.browserUploadProgress.total
              ? copy.uploadStaged
              : `${copy.uploadingToServer} ${formatBytes(props.browserUploadProgress.loaded)} / ${formatBytes(props.browserUploadProgress.total)}`}
          /></div> : null}
        </div>
      </div>

      <div className="transfer-history-section">
        <div className="section-heading"><div><h2>{copy.sendHistory}</h2><p>{copy.sendHistoryHint}</p></div><span className="count-badge">{props.transfers.length}</span></div>
        {props.transfers.length > 0 ? <div className="transfer-list">{props.transfers.slice(0, 12).map((transfer) => <OutgoingTransferRow key={transfer.id} transfer={transfer} copy={copy} />)}</div> : <div className="compact-empty"><Clock3 size={19} aria-hidden="true" /><span>{copy.noSendHistory}</span></div>}
      </div>
    </section>
  );
}

function FileRow({ copy, file, onRemove }: { copy: Record<string, string>; file: File; onRemove: () => void }) {
  return <div className="file-row"><div className="file-type-icon"><File size={17} aria-hidden="true" /></div><div className="file-row-copy"><strong title={file.name}>{file.name}</strong><span>{formatBytes(file.size)}</span></div><button className="icon-button tiny" type="button" title={copy.removeFile} aria-label={`${copy.removeFile}: ${file.name}`} onClick={onRemove}><X size={15} aria-hidden="true" /></button></div>;
}

function DeviceCard({ device, selected, copy, onSelect }: { device: DeviceInfo; selected: boolean; copy: Record<string, string>; onSelect: () => void }) {
  const Icon = device.deviceType === 'mobile' ? Smartphone : device.deviceType === 'server' ? Server : device.deviceType === 'web' ? Monitor : Laptop;
  const source = device.sourceInterfaceLabel ?? device.sourceInterface;
  return <button type="button" className={`device-card ${selected ? 'selected' : ''}`} onClick={onSelect} aria-pressed={selected}><span className="device-icon"><Icon size={20} aria-hidden="true" /></span><span className="device-card-copy"><strong>{device.alias}</strong><span>{device.ip ?? 'LocalSend'} · {device.deviceModel ?? device.deviceType ?? 'device'}</span>{source ? <small>{copy.viaInterface} {source}</small> : null}</span><span className={`device-check ${selected ? 'visible' : ''}`}><Check size={16} aria-hidden="true" /></span></button>;
}

function ProgressBar({ value, total, label }: { value: number; total: number; label: string }) {
  const percent = total > 0 ? Math.min(100, Math.round((value / total) * 100)) : 0;
  return <div className="progress-group"><div className="progress-meta"><span>{label}</span><strong>{percent}%</strong></div><div className="progress-track" role="progressbar" aria-label={label} aria-valuetext={`${percent}%`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}><span style={{ transform: `scaleX(${percent / 100})` }} /></div></div>;
}

function OutgoingTransferRow({ transfer, copy }: { transfer: OutgoingTransfer; copy: Record<string, string> }) {
  const isText = transfer.isClipboard;
  const active = transfer.status === 'preparing' || transfer.status === 'sending';
  const StatusIcon = transfer.status === 'completed' ? CheckCircle2 : transfer.status === 'failed' ? CircleAlert : RefreshCw;
  const statusLabel = copy[`transferStatus${transfer.status[0].toUpperCase()}${transfer.status.slice(1)}`];
  return <article className="transfer-row">
    <div className={`transfer-kind-icon ${isText ? 'text' : ''}`}>{isText ? <FileText size={18} aria-hidden="true" /> : <File size={18} aria-hidden="true" />}</div>
    <div className="transfer-row-main">
      <div className="transfer-title"><strong>{isText ? copy.clipboardText : transfer.fileNames.length === 1 ? transfer.fileNames[0] : `${transfer.fileNames.length} ${copy.files}`}</strong><span className={`transfer-status ${transfer.status}`}><StatusIcon size={14} className={active ? 'spin' : ''} aria-hidden="true" />{statusLabel}</span></div>
      <p className="transfer-detail">{copy.to} {transfer.targetAlias} · {formatTime(transfer.createdAt)} · {formatBytes(transfer.totalBytes)}</p>
      {active ? <ProgressBar value={transfer.transferredBytes} total={transfer.totalBytes} label={`${copy.sending} ${formatBytes(transfer.transferredBytes)} / ${formatBytes(transfer.totalBytes)}`} /> : null}
      {transfer.error ? <p className="transfer-error">{transfer.error}</p> : null}
    </div>
  </article>;
}

function ReceiveView({ copy, status, pending, history, incomingTransfers, onDecision }: { copy: Record<string, string>; status: StatusResponse | null; pending: PendingTransfer | null; history: ReceivedFile[]; incomingTransfers: IncomingTransfer[]; onDecision: (decision: 'accept' | 'reject') => void }) {
  return <section className="workspace">
    <div className="page-intro compact-intro"><div><p className="eyebrow"><Inbox size={14} aria-hidden="true" /> {copy.inboxLabel.toUpperCase()}</p><h1>{copy.incoming}</h1><p className="page-subhead">{copy.receiveSubhead}</p></div></div>
    <div className="receive-grid">
      <div className="receive-main">
        {pending ? <div className="pending-panel"><div className="pending-header"><div className="sender-avatar"><Laptop size={20} aria-hidden="true" /></div><div><span className="eyebrow">{copy.waiting}</span><h2>{pending.sender.alias}</h2><p>{copy.from} {pending.sender.ip ?? 'LAN'} · {pending.files.length} {copy.selectedFiles.toLowerCase()}</p></div></div><div className="pending-files">{pending.files.map((file) => <div key={file.id} className="pending-file"><File size={16} aria-hidden="true" /><span>{file.name}</span><span>{formatBytes(file.size)}</span></div>)}</div><div className="pending-total"><span>{copy.selectedFiles}</span><strong>{formatBytes(pending.totalBytes)}</strong></div><div className="pending-actions"><button className="secondary-button danger-outline" type="button" onClick={() => onDecision('reject')}><X size={17} aria-hidden="true" />{copy.reject}</button><button className="primary-button" type="button" onClick={() => onDecision('accept')}><Check size={17} aria-hidden="true" />{copy.accept}</button></div></div> : null}
        <div className="incoming-activity-panel">
          <div className="section-heading"><div><h2>{copy.receiveActivity}</h2><p>{copy.receiveActivityHint}</p></div><span className="count-badge">{incomingTransfers.length}</span></div>
          {incomingTransfers.length > 0 ? <div className="incoming-transfer-list">{incomingTransfers.slice(0, 12).map((transfer) => <IncomingTransferRow key={transfer.id} transfer={transfer} copy={copy} />)}</div> : !pending ? <div className="compact-empty"><Inbox size={20} aria-hidden="true" /><span>{copy.noPending}</span></div> : null}
        </div>
      </div>
      <aside className="receive-side"><div className="side-panel"><div className="side-panel-heading"><h2>{copy.localNode}</h2><span className="status-tag"><span className="status-dot" />{copy.online}</span></div><div className="node-name">{status?.alias ?? copy.brand}</div><dl className="detail-list"><div><dt>{copy.protocol}</dt><dd>{status?.protocol?.toUpperCase() ?? 'HTTPS'}</dd></div><div><dt>{copy.port}</dt><dd>{status?.localsendPort ?? 53317}</dd></div><div><dt>{copy.downloads}</dt><dd title={status?.dataDirectory}>{status?.dataDirectory ?? '/data/downloads'}</dd></div></dl></div><div className="side-panel history-panel"><div className="side-panel-heading"><h2>{copy.history}</h2><span className="count-badge">{history.length}</span></div>{history.length > 0 ? <div className="history-list">{history.slice(0, 8).map((file, index) => <div key={`${file.fileName}-${index}`} className="history-row"><div className="file-type-icon"><File size={16} aria-hidden="true" /></div><div className="file-row-copy"><strong title={file.fileName}>{file.fileName}</strong><span>{file.sender} · {formatTime(file.time)}</span></div><span className="history-size">{formatBytes(file.size)}</span></div>)}</div> : <p className="empty-copy">{copy.noHistory}</p>}</div></aside>
    </div>
  </section>;
}

function IncomingTransferRow({ transfer, copy }: { transfer: IncomingTransfer; copy: Record<string, string> }) {
  const active = transfer.status === 'waiting' || transfer.status === 'receiving';
  const StatusIcon = transfer.status === 'completed' ? CheckCircle2 : transfer.status === 'failed' ? CircleAlert : RefreshCw;
  const statusLabel = copy[`incomingStatus${transfer.status[0].toUpperCase()}${transfer.status.slice(1)}`];
  return <article className="transfer-row incoming-transfer-row"><div className="transfer-kind-icon incoming"><File size={18} aria-hidden="true" /></div><div className="transfer-row-main"><div className="transfer-title"><strong title={transfer.fileName}>{transfer.fileName}</strong><span className={`transfer-status ${transfer.status}`}><StatusIcon size={14} className={active && transfer.status === 'receiving' ? 'spin' : ''} aria-hidden="true" />{statusLabel}</span></div><p className="transfer-detail">{copy.from} {transfer.senderAlias} · {formatTime(transfer.createdAt)} · {formatBytes(transfer.totalBytes)}</p>{active ? <ProgressBar value={transfer.transferredBytes} total={transfer.totalBytes} label={`${copy.receiving} ${formatBytes(transfer.transferredBytes)} / ${formatBytes(transfer.totalBytes)}`} /> : null}{transfer.error ? <p className="transfer-error">{transfer.error}</p> : null}</div></article>;
}

function SettingsView({ copy, locale, theme, status, onLocale, onTheme, onError, onNotice }: { copy: Record<string, string>; locale: Locale; theme: Theme; status: StatusResponse | null; onLocale: (locale: Locale) => void; onTheme: (theme: Theme) => void; onError: (message: string | null) => void; onNotice: (message: string | null) => void }) {
  const [showAdvanced, setShowAdvanced] = useState(() => localStorage.getItem('localsendy-advanced-settings') === 'true');
  const [networks, setNetworks] = useState<NetworkSettings | null>(null);
  const [draftMode, setDraftMode] = useState<NetworkMode>('all');
  const [draftSelected, setDraftSelected] = useState<Set<string>>(new Set());
  const [draftLabels, setDraftLabels] = useState<Record<string, string>>({});
  const [isLoadingNetworks, setIsLoadingNetworks] = useState(true);
  const [isSavingNetworks, setIsSavingNetworks] = useState(false);
  const [storage, setStorage] = useState<StorageSettings | null>(null);
  const [isLoadingStorage, setIsLoadingStorage] = useState(true);
  const [isStoragePickerOpen, setIsStoragePickerOpen] = useState(false);
  const [environment, setEnvironment] = useState<EnvironmentSettings | null>(null);
  const [draftAutoAccept, setDraftAutoAccept] = useState(false);
  const [draftAlias, setDraftAlias] = useState('');
  const [draftAliasLocale, setDraftAliasLocale] = useState<AliasLocale>('auto');
  const [isLoadingEnvironment, setIsLoadingEnvironment] = useState(true);
  const [isSavingEnvironment, setIsSavingEnvironment] = useState(false);
  const [isAutoAcceptConfirmOpen, setIsAutoAcceptConfirmOpen] = useState(false);

  function resetNetworkDraft(next: NetworkSettings) {
    setNetworks(next);
    setDraftMode(next.mode);
    setDraftSelected(new Set(next.selectedInterfaces));
    setDraftLabels(Object.fromEntries(next.interfaces.map((network) => [network.name, network.label ?? ''])));
  }

  useEffect(() => {
    let active = true;
    setIsLoadingNetworks(true);
    getNetworkSettings()
      .then((next) => {
        if (active) resetNetworkDraft(next);
      })
      .catch((requestError) => {
        if (active) onError(requestError instanceof Error ? requestError.message : copy.error);
      })
      .finally(() => {
        if (active) setIsLoadingNetworks(false);
      });
    return () => { active = false; };
  }, [copy.error, onError]);

  useEffect(() => {
    let active = true;
    setIsLoadingEnvironment(true);
    getEnvironmentSettings()
      .then((next) => {
        if (!active) return;
        setEnvironment(next);
        setDraftAutoAccept(next.autoAccept);
        setDraftAlias(next.alias);
        setDraftAliasLocale(next.aliasLocale);
      })
      .catch((requestError) => {
        if (active) onError(requestError instanceof Error ? requestError.message : copy.error);
      })
      .finally(() => {
        if (active) setIsLoadingEnvironment(false);
      });
    return () => { active = false; };
  }, [copy.error, onError]);

  useEffect(() => {
    let active = true;
    setIsLoadingStorage(true);
    getStorageSettings()
      .then((next) => {
        if (active) setStorage(next);
      })
      .catch((requestError) => {
        if (active) onError(requestError instanceof Error ? requestError.message : copy.error);
      })
      .finally(() => {
        if (active) setIsLoadingStorage(false);
      });
    return () => { active = false; };
  }, [copy.error, onError]);

  const selectableInterfaces = networks?.interfaces.filter((network) => network.discoveryCapable) ?? [];
  const selectedCount = draftMode === 'all'
    ? networks?.interfaces.filter((network) => network.selected).length ?? 0
    : selectableInterfaces.filter((network) => draftSelected.has(network.name)).length;
  const savedSelection = networks?.selectedInterfaces ?? [];
  const selectionChanged = draftMode === 'selected'
    && [...draftSelected].sort().join('\0') !== [...savedSelection].sort().join('\0');
  const labelsChanged = Boolean(networks && networks.interfaces.some((network) => (network.label ?? '') !== (draftLabels[network.name] ?? '').trim()));
  const hasChanges = Boolean(networks && (draftMode !== networks.mode || selectionChanged || labelsChanged));
  const hasValidSelection = draftMode === 'all' || selectedCount > 0;

  function handleAllNetworks(enabled: boolean) {
    if (!enabled && draftSelected.size === 0) {
      setDraftSelected(new Set(selectableInterfaces.map((network) => network.name)));
    }
    setDraftMode(enabled ? 'all' : 'selected');
  }

  function toggleNetwork(name: string) {
    setDraftSelected((current) => {
      const next = new Set(current);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  function updateNetworkLabel(name: string, label: string) {
    setDraftLabels((current) => ({ ...current, [name]: label }));
  }

  async function refreshNetworks() {
    setIsLoadingNetworks(true);
    onError(null);
    try {
      resetNetworkDraft(await getNetworkSettings());
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsLoadingNetworks(false);
    }
  }

  async function applyNetworks() {
    if (!hasChanges || !hasValidSelection) return;
    setIsSavingNetworks(true);
    onError(null);
    try {
      const next = await updateNetworkSettings(draftMode, [...draftSelected], draftLabels);
      resetNetworkDraft(next);
      onNotice(copy.networksApplied);
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsSavingNetworks(false);
    }
  }

  const environmentChanged = Boolean(environment && (
    draftAutoAccept !== environment.autoAccept
    || draftAlias.trim() !== environment.alias
    || draftAliasLocale !== environment.aliasLocale
  ));

  function generateRandomAlias() {
    setDraftAlias('');
    setDraftAliasLocale(locale === 'zh-CN' || locale === 'zh-TW' ? locale : 'en');
  }

  function toggleAutoAccept(enabled: boolean) {
    if (enabled && !draftAutoAccept) {
      setIsAutoAcceptConfirmOpen(true);
      return;
    }
    setDraftAutoAccept(enabled);
  }

  function confirmAutoAccept() {
    setDraftAutoAccept(true);
    setIsAutoAcceptConfirmOpen(false);
  }

  async function applyEnvironment() {
    if (!environmentChanged) return;
    setIsSavingEnvironment(true);
    onError(null);
    try {
      const next = await updateEnvironmentSettings({
        autoAccept: draftAutoAccept,
        alias: draftAlias,
        aliasLocale: draftAliasLocale,
      });
      setEnvironment(next);
      setDraftAutoAccept(next.autoAccept);
      setDraftAlias(next.alias);
      setDraftAliasLocale(next.aliasLocale);
      onNotice(copy.environmentApplied);
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsSavingEnvironment(false);
    }
  }

  const deviceNetworks = networks?.interfaces.filter((network) => network.kind !== 'bridge' && network.kind !== 'virtual') ?? [];
  const virtualNetworks = networks?.interfaces.filter((network) => network.kind === 'bridge' || network.kind === 'virtual') ?? [];
  const deviceTypeLabel = status?.deviceType
    ? copy[`deviceType${status.deviceType[0].toUpperCase()}${status.deviceType.slice(1)}`]
    : '—';

  function toggleAdvanced(enabled: boolean) {
    setShowAdvanced(enabled);
    localStorage.setItem('localsendy-advanced-settings', String(enabled));
  }

  return (
    <section className="workspace settings-workspace">
      <div className="page-intro compact-intro"><div><p className="eyebrow"><SettingsIcon size={14} aria-hidden="true" /> {copy.controlPlane.toUpperCase()}</p><h1>{copy.settings}</h1><p className="page-subhead">{copy.tagline}</p></div></div>
      <div className="settings-grid">
        <div className="settings-section">
          <div className="section-heading"><div><h2>{copy.appearance}</h2><p>{copy.theme} · {copy.language}</p></div></div>
          <div className="setting-row"><div className="setting-label"><Sun size={18} aria-hidden="true" /><span>{copy.theme}</span></div><div className="segmented-control">{([['system', Monitor, copy.system], ['light', Sun, copy.light], ['dark', Moon, copy.dark]] as const).map(([value, Icon, label]) => <button key={value} type="button" className={theme === value ? 'active' : ''} onClick={() => onTheme(value)}><Icon size={16} aria-hidden="true" /><span>{label}</span></button>)}</div></div>
          <div className="setting-row"><div className="setting-label"><Languages size={18} aria-hidden="true" /><span>{copy.language}</span></div><select className="setting-select" aria-label={copy.language} value={locale} onChange={(event) => onLocale(event.target.value as Locale)}><option value="en">English</option><option value="zh-CN">简体中文</option><option value="zh-TW">繁體中文</option></select></div>
        </div>

        <div className="settings-section">
          <div className="section-heading"><div><h2>{copy.storage}</h2><p>{copy.downloads}</p></div><HardDrive size={21} className="section-icon" aria-hidden="true" /></div>
          <div className="storage-control">
            <div className="storage-path"><FolderOpen size={18} aria-hidden="true" /><code>{storage?.resolvedPath ?? status?.dataDirectory ?? '/data/downloads'}</code></div>
            <button className="secondary-button storage-choose-button" type="button" disabled={isLoadingStorage} onClick={() => setIsStoragePickerOpen(true)}>
              {isLoadingStorage ? <RefreshCw size={17} className="spin" aria-hidden="true" /> : <FolderOpen size={17} aria-hidden="true" />}
              {copy.chooseDirectory}
            </button>
          </div>
          <p className="storage-root-hint">{copy.storageRoot}: <code>{storage?.root ?? copy.loading}</code></p>
        </div>

        <div className="settings-section environment-settings-section">
          <div className="section-heading"><div><h2>{copy.environmentVariables}</h2><p>{copy.environmentVariablesHint}</p></div><Server size={21} className="section-icon" aria-hidden="true" /></div>
          {isLoadingEnvironment ? <div className="network-loading"><RefreshCw size={18} className="spin" aria-hidden="true" /><span>{copy.loading}</span></div> : <>
            <label className="network-mode-row environment-toggle-row">
              <span className="network-mode-copy"><strong>{copy.autoAccept}</strong><small>{copy.autoAcceptHint}</small></span>
              <span className="network-mode-meta"><span>{draftAutoAccept ? copy.enabled : copy.disabled}</span><input className="sr-only" type="checkbox" checked={draftAutoAccept} aria-describedby="auto-accept-warning" onChange={(event) => toggleAutoAccept(event.target.checked)} disabled={isSavingEnvironment} /><span className="switch-track" aria-hidden="true"><span className="switch-thumb" /></span></span>
            </label>
            <p id="auto-accept-warning" className="environment-security-note"><CircleAlert size={16} aria-hidden="true" /><span>{copy.autoAcceptWarning}</span></p>
            <div className="environment-form">
              <label className="environment-field" htmlFor="environment-alias"><span>{copy.alias}</span><input id="environment-alias" value={draftAlias} onChange={(event) => setDraftAlias(event.target.value)} placeholder={copy.aliasPlaceholder} maxLength={64} disabled={isSavingEnvironment} /></label>
              <div className="environment-field"><label htmlFor="environment-alias-locale">{copy.aliasLocale}</label><div className="environment-alias-actions"><select id="environment-alias-locale" className="setting-select" value={draftAliasLocale} onChange={(event) => setDraftAliasLocale(event.target.value as AliasLocale)} disabled={isSavingEnvironment}><option value="auto">{copy.aliasLocaleAuto}</option><option value="en">English</option><option value="zh-CN">简体中文</option><option value="zh-TW">繁體中文</option></select><button className="secondary-button" type="button" onClick={generateRandomAlias} disabled={isSavingEnvironment}>{copy.randomAlias}</button></div></div>
            </div>
            <div className="environment-actions"><span className="network-runtime-note"><ShieldCheck size={15} aria-hidden="true" />{copy.environmentVariablesHint}</span><button className="primary-button" type="button" disabled={!environmentChanged || isSavingEnvironment} onClick={applyEnvironment}>{isSavingEnvironment ? <RefreshCw size={17} className="spin" aria-hidden="true" /> : <Check size={17} aria-hidden="true" />}{isSavingEnvironment ? copy.saving : copy.saveChanges}</button></div>
          </>}
        </div>

        <div className="settings-section advanced-settings-section deployment-section">
          <label className="advanced-toggle-row">
            <span className="network-mode-copy"><strong>{copy.advancedSettings}</strong><small>{copy.advancedSettingsHint}</small></span>
            <span className="network-mode-meta"><input className="sr-only" type="checkbox" checked={showAdvanced} onChange={(event) => toggleAdvanced(event.target.checked)} /><span className="switch-track" aria-hidden="true"><span className="switch-thumb" /></span></span>
          </label>
          {showAdvanced ? <>
            <div className="info-grid advanced-info-grid">
              <InfoItem icon={<Server size={17} />} label={copy.alias} value={status?.alias ?? '—'} />
              <InfoItem icon={<Monitor size={17} />} label={copy.deviceType} value={deviceTypeLabel} />
              <InfoItem icon={<Laptop size={17} />} label={copy.deviceModel} value={status?.deviceModel ?? '—'} />
              <InfoItem icon={<ShieldCheck size={17} />} label={copy.encryption} value={status?.protocol?.toUpperCase() ?? 'HTTPS'} />
              <InfoItem icon={<Wifi size={17} />} label={copy.ipv4Multicast} value={status?.multicastIpv4 ?? '224.0.0.167'} />
              <InfoItem icon={<Wifi size={17} />} label={copy.ipv6Multicast} value={status?.multicastIpv6 ?? 'ff12::fd3a:e420'} />
              <InfoItem icon={<RefreshCw size={17} />} label={copy.discoveryInterval} value={`${status?.discoveryIntervalSeconds ?? 30} ${copy.seconds}`} />
              <InfoItem icon={<UploadCloud size={17} />} label={copy.uploadLimit} value={formatBytes(status?.maxUploadBytes ?? 10_737_418_240)} />
            </div>
            <p className="advanced-settings-note">{copy.managedByEnvironment}</p>
          </> : null}
        </div>

        <div className="settings-section network-settings-section deployment-section">
          <div className="section-heading network-section-heading">
            <div><h2>{copy.discoveryInterfaces}</h2><p>{copy.discoveryInterfacesHint}</p></div>
            <button className="icon-button outlined" type="button" title={copy.refresh} aria-label={copy.refresh} onClick={refreshNetworks} disabled={isLoadingNetworks || isSavingNetworks}><RefreshCw size={17} className={isLoadingNetworks ? 'spin' : ''} aria-hidden="true" /></button>
          </div>

          <div className="info-grid network-info-grid">
            <InfoItem icon={<Server size={17} />} label={copy.port} value={String(status?.localsendPort ?? 53317)} />
            <InfoItem icon={<ShieldCheck size={17} />} label={copy.protocol} value={status?.protocol?.toUpperCase() ?? 'HTTPS'} />
            <InfoItem icon={<Wifi size={17} />} label={copy.nearbyDevices} value={String(status?.nearbyDevices ?? 0)} />
            <InfoItem icon={<HardDrive size={17} />} label={copy.autoAccept} value={status?.autoAccept ? copy.enabled : copy.disabled} />
          </div>

          <label className="network-mode-row">
            <span className="network-mode-copy"><strong>{copy.allNetworks}</strong><small>{draftMode === 'all' ? copy.allNetworksHint : copy.chooseNetworksHint}</small></span>
            <span className="network-mode-meta"><span><strong>{selectedCount}</strong> / {selectableInterfaces.length} {copy.availableNetworks}</span><input className="sr-only" type="checkbox" checked={draftMode === 'all'} onChange={(event) => handleAllNetworks(event.target.checked)} disabled={isLoadingNetworks || isSavingNetworks} /><span className="switch-track" aria-hidden="true"><span className="switch-thumb" /></span></span>
          </label>

          {isLoadingNetworks ? <div className="network-loading"><RefreshCw size={18} className="spin" aria-hidden="true" /><span>{copy.scanning}</span></div> : null}
          {!isLoadingNetworks && networks?.interfaces.length === 0 ? <div className="network-empty"><Wifi size={20} aria-hidden="true" /><span>{copy.noNetworkInterfaces}</span></div> : null}
          {!isLoadingNetworks && deviceNetworks.length > 0 ? <NetworkInterfaceGroup title={copy.deviceNetworks} hint={copy.deviceNetworksHint} networks={deviceNetworks} mode={draftMode} selected={draftSelected} labels={draftLabels} copy={copy} disabled={isSavingNetworks} onToggle={toggleNetwork} onLabel={updateNetworkLabel} /> : null}
          {!isLoadingNetworks && virtualNetworks.length > 0 ? <NetworkInterfaceGroup title={copy.virtualNetworks} hint={copy.virtualNetworksHint} networks={virtualNetworks} mode={draftMode} selected={draftSelected} labels={draftLabels} copy={copy} disabled={isSavingNetworks} collapsible onToggle={toggleNetwork} onLabel={updateNetworkLabel} /> : null}

          <div className="network-actions">
            <div className="network-runtime-note"><ShieldCheck size={15} aria-hidden="true" /><span>{hasChanges ? copy.networkChangesPending : copy.networkConfigSaved}</span></div>
            <div className="network-action-buttons">
              <button className="secondary-button" type="button" disabled={!hasChanges || isSavingNetworks} onClick={() => networks && resetNetworkDraft(networks)}>{copy.discardChanges}</button>
              <button className="primary-button" type="button" disabled={!hasChanges || !hasValidSelection || isSavingNetworks} onClick={applyNetworks}>{isSavingNetworks ? <RefreshCw size={17} className="spin" aria-hidden="true" /> : <Check size={17} aria-hidden="true" />}{isSavingNetworks ? copy.applyingNetworks : copy.applyNetworks}</button>
            </div>
          </div>
        </div>

        <div className="settings-section deployment-section"><div className="section-heading"><div><h2>{copy.deployment}</h2><p>Docker</p></div><Server size={21} className="section-icon" aria-hidden="true" /></div><p className="deployment-copy">{copy.deploymentHint}</p><div className="setting-row readonly-setting"><div className="setting-label"><ShieldCheck size={18} aria-hidden="true" /><span>{copy.version}</span></div><strong>v{status?.version ?? '0.3.0'}</strong></div></div>
      </div>
      {storage ? <StorageDirectoryDialog
        open={isStoragePickerOpen}
        storage={storage}
        copy={copy}
        onClose={() => setIsStoragePickerOpen(false)}
        onSelected={(next) => {
          setStorage(next);
          setIsStoragePickerOpen(false);
          onNotice(copy.storageUpdated);
        }}
        onError={onError}
      /> : null}
      <ConfirmDialog
        open={isAutoAcceptConfirmOpen}
        title={copy.autoAcceptDialogTitle}
        description={copy.autoAcceptConfirm}
        confirmLabel={copy.enableAutoAccept}
        cancelLabel={copy.cancel}
        icon={<CircleAlert size={20} aria-hidden="true" />}
        tone="warning"
        onCancel={() => setIsAutoAcceptConfirmOpen(false)}
        onConfirm={confirmAutoAccept}
      />
      <div className="settings-footnote"><ShieldCheck size={15} aria-hidden="true" /> {copy.privacyFootnote}</div>
    </section>
  );
}

function StorageDirectoryDialog({ open, storage, copy, onClose, onSelected, onError }: { open: boolean; storage: StorageSettings; copy: Record<string, string>; onClose: () => void; onSelected: (storage: StorageSettings) => void; onError: (message: string | null) => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [newDirectoryName, setNewDirectoryName] = useState('');

  const loadDirectory = useCallback(async (path: string) => {
    setIsLoading(true);
    onError(null);
    try {
      setListing(await listStorageDirectories(path));
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsLoading(false);
    }
  }, [copy.error, onError]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
      setIsCreating(false);
      setNewDirectoryName('');
      void loadDirectory(storage.subdirectory);
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [loadDirectory, open, storage.subdirectory]);

  const segments = listing?.path ? listing.path.split('/') : [];

  async function chooseCurrentDirectory() {
    if (!listing) return;
    setIsSaving(true);
    onError(null);
    try {
      onSelected(await updateStorageSettings(listing.path));
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsSaving(false);
    }
  }

  async function createDirectory() {
    if (!listing || !newDirectoryName.trim()) return;
    setIsSaving(true);
    onError(null);
    try {
      setListing(await createStorageDirectory(listing.path, newDirectoryName.trim()));
      setNewDirectoryName('');
      setIsCreating(false);
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <dialog ref={dialogRef} className="directory-dialog" aria-labelledby="directory-dialog-title" onCancel={(event) => { event.preventDefault(); onClose(); }} onClose={onClose}>
      <div className="directory-dialog-header">
        <div><p className="eyebrow">{copy.storageRoot}</p><h2 id="directory-dialog-title">{copy.chooseDirectory}</h2></div>
        <button className="icon-button" type="button" aria-label={copy.close} title={copy.close} onClick={onClose}><X size={18} aria-hidden="true" /></button>
      </div>
      <div className="directory-toolbar">
        <button className="icon-button outlined" type="button" aria-label={copy.parentDirectory} title={copy.parentDirectory} disabled={isLoading || listing?.parent === undefined} onClick={() => listing?.parent !== undefined && void loadDirectory(listing.parent)}><ArrowLeft size={18} aria-hidden="true" /></button>
        <nav className="directory-breadcrumbs" aria-label={copy.currentDirectory}>
          <button type="button" onClick={() => void loadDirectory('')}>{copy.rootDirectory}</button>
          {segments.map((segment, index) => {
            const path = segments.slice(0, index + 1).join('/');
            return <span key={path}><ChevronRight size={14} aria-hidden="true" /><button type="button" onClick={() => void loadDirectory(path)}>{segment}</button></span>;
          })}
        </nav>
        <button className="icon-button outlined" type="button" aria-label={copy.newDirectory} title={copy.newDirectory} disabled={isLoading || isSaving} onClick={() => setIsCreating((current) => !current)}><FolderPlus size={18} aria-hidden="true" /></button>
      </div>
      {isCreating ? <form className="new-directory-form" onSubmit={(event) => { event.preventDefault(); void createDirectory(); }}>
        <label htmlFor="new-directory-name">{copy.directoryName}</label>
        <div><input id="new-directory-name" autoFocus value={newDirectoryName} onChange={(event) => setNewDirectoryName(event.target.value)} placeholder={copy.directoryNamePlaceholder} /><button className="secondary-button" type="submit" disabled={!newDirectoryName.trim() || isSaving}>{copy.create}</button></div>
      </form> : null}
      <div className="directory-list" aria-busy={isLoading}>
        {isLoading ? <div className="directory-state"><RefreshCw size={20} className="spin" aria-hidden="true" /><span>{copy.loading}</span></div> : null}
        {!isLoading && listing?.directories.length === 0 ? <div className="directory-state"><FolderOpen size={22} aria-hidden="true" /><span>{copy.noSubdirectories}</span></div> : null}
        {!isLoading ? listing?.directories.map((directory) => {
          const path = listing.path ? `${listing.path}/${directory}` : directory;
          return <button className="directory-row" type="button" key={directory} onClick={() => void loadDirectory(path)}><span className="directory-icon"><FolderOpen size={19} aria-hidden="true" /></span><strong>{directory}</strong><ChevronRight size={17} aria-hidden="true" /></button>;
        }) : null}
      </div>
      <div className="directory-dialog-footer">
        <div><span>{copy.currentDirectory}</span><code>{listing?.path || copy.rootDirectory}</code></div>
        <div><button className="secondary-button" type="button" onClick={onClose}>{copy.cancel}</button><button className="primary-button" type="button" disabled={!listing || isLoading || isSaving} onClick={() => void chooseCurrentDirectory()}>{isSaving ? <RefreshCw size={17} className="spin" aria-hidden="true" /> : <Check size={17} aria-hidden="true" />}{copy.selectCurrentDirectory}</button></div>
      </div>
    </dialog>
  );
}

function NetworkInterfaceGroup({ title, hint, networks, mode, selected, labels, copy, disabled, collapsible = false, onToggle, onLabel }: { title: string; hint: string; networks: NetworkInterfaceInfo[]; mode: NetworkMode; selected: Set<string>; labels: Record<string, string>; copy: Record<string, string>; disabled: boolean; collapsible?: boolean; onToggle: (name: string) => void; onLabel: (name: string, label: string) => void }) {
  const rows = <div className="network-interface-grid">{networks.map((network) => <NetworkInterfaceRow key={network.name} network={network} mode={mode} selected={mode === 'all' ? network.selected : selected.has(network.name)} label={labels[network.name] ?? ''} copy={copy} disabled={disabled} onToggle={() => onToggle(network.name)} onLabel={(label) => onLabel(network.name, label)} />)}</div>;
  const heading = <div className="network-group-heading"><div><h3>{title}</h3><p>{hint}</p></div><span>{networks.length}</span></div>;
  if (collapsible) {
    return <details className="network-group collapsible" open={mode === 'selected' && networks.some((network) => selected.has(network.name)) ? true : undefined}><summary><ChevronRight size={16} aria-hidden="true" />{heading}</summary>{rows}</details>;
  }
  return <section className="network-group" aria-label={title}>{heading}{rows}</section>;
}

function NetworkInterfaceRow({ network, mode, selected, label, copy, disabled, onToggle, onLabel }: { network: NetworkInterfaceInfo; mode: NetworkMode; selected: boolean; label: string; copy: Record<string, string>; disabled: boolean; onToggle: () => void; onLabel: (label: string) => void }) {
  const checked = network.discoveryCapable && selected;
  const controlDisabled = disabled || mode === 'all' || !network.discoveryCapable;
  const Icon = network.kind === 'wifi' ? Wifi : network.kind === 'bridge' || network.kind === 'virtual' ? Server : network.kind === 'tunnel' ? ShieldCheck : Monitor;
  const kindLabel = copy[`interfaceKind${network.kind[0].toUpperCase()}${network.kind.slice(1)}`];
  const capability = network.coveredBy && mode === 'all'
    ? copy.sameNetworkCovered.replace('{interface}', network.coveredBy)
    : network.ipv4Discovery && network.ipv6Discovery ? copy.dualStackMulticast : network.ipv6Discovery ? copy.ipv6Multicast : network.ipv4Discovery ? copy.ipv4Multicast : network.pointToPoint ? copy.pointToPoint : copy.manualOnly;
  return (
    <div className={`network-interface-row ${checked ? 'selected' : ''} ${mode === 'all' ? 'automatic' : ''} ${network.coveredBy && mode === 'all' ? 'covered' : ''} ${!network.discoveryCapable ? 'manual-only' : ''}`} title={!network.discoveryCapable ? copy.manualOnlyHint : undefined}>
      <span className="network-interface-icon"><Icon size={18} strokeWidth={1.8} aria-hidden="true" /></span>
      <span className="network-interface-content">
        <span className="network-interface-title"><strong>{network.name}</strong><span>{kindLabel}</span></span>
        <span className="network-addresses">
          {network.ipv4Addresses.map((address) => <code key={`v4-${address}`}>{address}</code>)}
          {network.ipv6Addresses.map((address) => <code key={`v6-${address}`} className="ipv6-address">{address}</code>)}
        </span>
        <span className={`network-capability ${network.discoveryCapable ? 'capable' : ''}`}><span className="status-dot" aria-hidden="true" />{capability}</span>
        <label className="network-interface-label"><span>{copy.interfaceLabel}</span><input type="text" maxLength={64} value={label} placeholder={copy.interfaceLabelPlaceholder} disabled={disabled} onChange={(event) => onLabel(event.target.value)} /></label>
      </span>
      <label className="network-checkbox-wrap" aria-label={`${network.name}: ${capability}`}><input className="sr-only" type="checkbox" checked={checked} disabled={controlDisabled} onChange={onToggle} /><span className="network-checkbox" aria-hidden="true">{checked ? <Check size={15} /> : null}</span></label>
    </div>
  );
}

function InfoItem({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return <div className="info-item"><span className="info-icon">{icon}</span><span><small>{label}</small><strong>{value}</strong></span></div>;
}
