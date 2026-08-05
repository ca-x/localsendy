import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ChangeEvent, DragEvent, ReactNode } from 'react';
import {
  Check,
  ChevronRight,
  CircleAlert,
  File,
  FolderOpen,
  HardDrive,
  Inbox,
  Languages,
  Laptop,
  Monitor,
  Moon,
  RefreshCw,
  Send as SendIcon,
  Server,
  Settings as SettingsIcon,
  ShieldCheck,
  Smartphone,
  Sun,
  UploadCloud,
  Wifi,
  X,
} from 'lucide-react';
import {
  decidePending,
  getDevices,
  getHistory,
  getNetworkSettings,
  getPending,
  getStatus,
  getTransfers,
  probeDevice,
  scanDevices,
  sendFiles,
  updateNetworkSettings,
} from './api';
import { detectLocale, messages } from './i18n';
import { formatBytes, formatTime } from './format';
import type {
  DeviceInfo,
  Locale,
  NetworkInterfaceInfo,
  NetworkMode,
  NetworkSettings,
  OutgoingTransfer,
  PendingTransfer,
  ReceivedFile,
  StatusResponse,
  Tab,
} from './types';

type Theme = 'system' | 'light' | 'dark';
type IconComponent = typeof SendIcon;

const navItems: Array<{ id: Tab; icon: IconComponent; label: string }> = [
  { id: 'send', icon: SendIcon, label: 'send' },
  { id: 'receive', icon: Inbox, label: 'receive' },
  { id: 'settings', icon: SettingsIcon, label: 'settings' },
];

export default function App() {
  const [locale, setLocale] = useState<Locale>(detectLocale);
  const [theme, setTheme] = useState<Theme>(() => (localStorage.getItem('localsendy-theme') as Theme) || 'system');
  const [activeTab, setActiveTab] = useState<Tab>('send');
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [pending, setPending] = useState<PendingTransfer | null>(null);
  const [history, setHistory] = useState<ReceivedFile[]>([]);
  const [transfers, setTransfers] = useState<OutgoingTransfer[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<DeviceInfo | null>(null);
  const [files, setFiles] = useState<File[]>([]);
  const [isScanning, setIsScanning] = useState(false);
  const [manualAddress, setManualAddress] = useState('');
  const [isProbing, setIsProbing] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const copy = useMemo(() => messages(locale), [locale]);

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextDevices, nextPending, nextHistory, nextTransfers] = await Promise.all([
        getStatus(),
        getDevices(),
        getPending(),
        getHistory(),
        getTransfers(),
      ]);
      setStatus(nextStatus);
      setDevices(nextDevices);
      setPending(nextPending);
      setHistory(nextHistory);
      setTransfers(nextTransfers);
      setSelectedDevice((current) =>
        current ? nextDevices.find((device) => device.fingerprint === current.fingerprint) ?? null : null,
      );
      setError(null);
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.error);
    }
  }, [copy.error]);

  useEffect(() => {
    refresh();
    const timer = window.setInterval(refresh, 2500);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem('localsendy-theme', theme);
  }, [theme]);

  useEffect(() => {
    document.documentElement.lang = locale;
    localStorage.setItem('localsendy-locale', locale);
  }, [locale]);

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
      setSelectedDevice(device);
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
    if (!selectedDevice || files.length === 0) return;
    setIsSending(true);
    setError(null);
    try {
      await sendFiles(selectedDevice, files);
      setFiles([]);
      setNotice(copy.transferComplete);
      await refresh();
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : copy.transferFailed);
    } finally {
      setIsSending(false);
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

  const nav = (id: Tab) => setActiveTab(id);

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
          <span className="version-label">v0.1.0</span>
        </div>
      </aside>

      <main id="main-content" className="main-content" tabIndex={-1}>
        <header className="topbar">
          <div className="topbar-status">
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

        {activeTab === 'send' ? (
          <SendView
            copy={copy}
            devices={devices}
            selectedDevice={selectedDevice}
            files={files}
            isScanning={isScanning}
            manualAddress={manualAddress}
            isProbing={isProbing}
            isSending={isSending}
            isDragging={isDragging}
            fileInputRef={fileInputRef}
            onScan={handleScan}
            onManualAddress={setManualAddress}
            onProbe={handleProbe}
            onSelectDevice={setSelectedDevice}
            onFileInput={handleFileInput}
            onDrop={handleDrop}
            onDragState={setIsDragging}
            onClearFiles={() => setFiles([])}
            onRemoveFile={(index) => setFiles((current) => current.filter((_, currentIndex) => currentIndex !== index))}
            onSend={handleSend}
          />
        ) : null}
        {activeTab === 'receive' ? <ReceiveView copy={copy} status={status} pending={pending} history={history} onDecision={handlePending} /> : null}
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
      <div className="brand-mark" aria-hidden="true"><SendIcon size={20} strokeWidth={2.2} /></div>
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
  selectedDevice: DeviceInfo | null;
  files: File[];
  isScanning: boolean;
  manualAddress: string;
  isProbing: boolean;
  isSending: boolean;
  isDragging: boolean;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  onScan: () => void;
  onManualAddress: (address: string) => void;
  onProbe: () => void;
  onSelectDevice: (device: DeviceInfo) => void;
  onFileInput: (event: ChangeEvent<HTMLInputElement>) => void;
  onDrop: (event: DragEvent<HTMLDivElement>) => void;
  onDragState: (state: boolean) => void;
  onClearFiles: () => void;
  onRemoveFile: (index: number) => void;
  onSend: () => void;
}) {
  const { copy } = props;
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

      <div className="send-grid">
        <div className="send-column">
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
            <input ref={props.fileInputRef} className="sr-only" type="file" multiple onChange={props.onFileInput} />
          </div>

          {props.files.length > 0 ? <div className="file-list-section"><div className="section-heading compact"><h2>{copy.selectedFiles}<span className="count-badge">{props.files.length}</span></h2></div><div className="file-list">{props.files.map((file, index) => <FileRow key={`${file.name}-${file.lastModified}`} copy={copy} file={file} onRemove={() => props.onRemoveFile(index)} />)}</div></div> : null}
        </div>

        <div className="devices-column">
          <div className="section-heading"><div><h2>{copy.nearbyDevices}</h2><p>{props.devices.length === 0 ? copy.noDevicesHint : `${props.devices.length} ${copy.online.toLowerCase()}`}</p></div><button className="icon-button outlined" type="button" title={props.isScanning ? copy.scanning : copy.scan} aria-label={props.isScanning ? copy.scanning : copy.scan} onClick={props.onScan} disabled={props.isScanning}>{<RefreshCw size={17} className={props.isScanning ? 'spin' : ''} aria-hidden="true" />}</button></div>
          {props.devices.length > 0 ? <div className="device-list">{props.devices.map((device) => <DeviceCard key={device.fingerprint} device={device} selected={props.selectedDevice?.fingerprint === device.fingerprint} copy={copy} onSelect={() => props.onSelectDevice(device)} />)}</div> : <div className="empty-device-state"><div className="empty-icon"><Wifi size={22} aria-hidden="true" /></div><strong>{copy.noDevices}</strong><span>{copy.noDevicesHint}</span><button className="secondary-button" type="button" onClick={props.onScan} disabled={props.isScanning}><RefreshCw size={17} className={props.isScanning ? 'spin' : ''} aria-hidden="true" />{props.isScanning ? copy.scanning : copy.scan}</button></div>}
          <div className="manual-target">
            <label htmlFor="manual-address">{copy.manualAddress}</label>
            <p>{copy.manualHint}</p>
            <div className="manual-target-row">
              <input id="manual-address" type="text" inputMode="url" autoComplete="off" placeholder="192.168.1.50[:53317]" value={props.manualAddress} onChange={(event) => props.onManualAddress(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') props.onProbe(); }} />
              <button className="secondary-button" type="button" disabled={props.isProbing || !props.manualAddress.trim()} onClick={props.onProbe}>{props.isProbing ? <RefreshCw size={17} className="spin" aria-hidden="true" /> : <Wifi size={17} aria-hidden="true" />}{props.isProbing ? copy.connecting : copy.connect}</button>
            </div>
          </div>
          <button className="primary-button send-cta" type="button" disabled={!props.selectedDevice || props.files.length === 0 || props.isSending} onClick={props.onSend}>{props.isSending ? <RefreshCw size={18} className="spin" aria-hidden="true" /> : <SendIcon size={18} aria-hidden="true" />}{props.isSending ? copy.sending : props.selectedDevice ? copy.sendFiles : copy.selectDevice}<ChevronRight size={17} aria-hidden="true" /></button>
        </div>
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

function ReceiveView({ copy, status, pending, history, onDecision }: { copy: Record<string, string>; status: StatusResponse | null; pending: PendingTransfer | null; history: ReceivedFile[]; onDecision: (decision: 'accept' | 'reject') => void }) {
  return <section className="workspace"><div className="page-intro compact-intro"><div><p className="eyebrow"><Inbox size={14} aria-hidden="true" /> {copy.inboxLabel.toUpperCase()}</p><h1>{copy.incoming}</h1><p className="page-subhead">{copy.waiting}</p></div></div><div className="receive-grid"><div className="receive-main">{pending ? <div className="pending-panel"><div className="pending-header"><div className="sender-avatar"><Laptop size={20} aria-hidden="true" /></div><div><span className="eyebrow">{copy.waiting}</span><h2>{pending.sender.alias}</h2><p>{copy.from} {pending.sender.ip ?? 'LAN'} · {pending.files.length} {copy.selectedFiles.toLowerCase()}</p></div></div><div className="pending-files">{pending.files.map((file) => <div key={file.id} className="pending-file"><File size={16} aria-hidden="true" /><span>{file.name}</span><span>{formatBytes(file.size)}</span></div>)}</div><div className="pending-total"><span>{copy.selectedFiles}</span><strong>{formatBytes(pending.totalBytes)}</strong></div><div className="pending-actions"><button className="secondary-button danger-outline" type="button" onClick={() => onDecision('reject')}><X size={17} aria-hidden="true" />{copy.reject}</button><button className="primary-button" type="button" onClick={() => onDecision('accept')}><Check size={17} aria-hidden="true" />{copy.accept}</button></div></div> : <div className="empty-panel"><div className="empty-icon"><Inbox size={22} aria-hidden="true" /></div><strong>{copy.noPending}</strong><span>{copy.waiting}</span></div>}</div><aside className="receive-side"><div className="side-panel"><div className="side-panel-heading"><h2>{copy.localNode}</h2><span className="status-tag"><span className="status-dot" />{copy.online}</span></div><div className="node-name">{status?.alias ?? copy.brand}</div><dl className="detail-list"><div><dt>{copy.protocol}</dt><dd>{status?.protocol?.toUpperCase() ?? 'HTTPS'}</dd></div><div><dt>{copy.port}</dt><dd>{status?.localsendPort ?? 53317}</dd></div><div><dt>{copy.downloads}</dt><dd title={status?.dataDirectory}>{status?.dataDirectory ?? '/data/downloads'}</dd></div></dl></div><div className="side-panel history-panel"><div className="side-panel-heading"><h2>{copy.history}</h2><span className="count-badge">{history.length}</span></div>{history.length > 0 ? <div className="history-list">{history.slice(0, 5).map((file, index) => <div key={`${file.fileName}-${index}`} className="history-row"><div className="file-type-icon"><File size={16} aria-hidden="true" /></div><div className="file-row-copy"><strong title={file.fileName}>{file.fileName}</strong><span>{file.sender} · {formatTime(file.time)}</span></div><span className="history-size">{formatBytes(file.size)}</span></div>)}</div> : <p className="empty-copy">{copy.noHistory}</p>}</div></aside></div></section>;
}

function SettingsView({ copy, locale, theme, status, onLocale, onTheme, onError, onNotice }: { copy: Record<string, string>; locale: Locale; theme: Theme; status: StatusResponse | null; onLocale: (locale: Locale) => void; onTheme: (theme: Theme) => void; onError: (message: string | null) => void; onNotice: (message: string | null) => void }) {
  const [networks, setNetworks] = useState<NetworkSettings | null>(null);
  const [draftMode, setDraftMode] = useState<NetworkMode>('all');
  const [draftSelected, setDraftSelected] = useState<Set<string>>(new Set());
  const [draftLabels, setDraftLabels] = useState<Record<string, string>>({});
  const [isLoadingNetworks, setIsLoadingNetworks] = useState(true);
  const [isSavingNetworks, setIsSavingNetworks] = useState(false);

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

  const selectableInterfaces = networks?.interfaces.filter((network) => network.discoveryCapable) ?? [];
  const selectedCount = draftMode === 'all'
    ? selectableInterfaces.length
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

  const deviceNetworks = networks?.interfaces.filter((network) => network.kind !== 'bridge' && network.kind !== 'virtual') ?? [];
  const virtualNetworks = networks?.interfaces.filter((network) => network.kind === 'bridge' || network.kind === 'virtual') ?? [];

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
          <div className="storage-path"><FolderOpen size={18} aria-hidden="true" /><code>{status?.dataDirectory ?? '/data/downloads'}</code></div>
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

        <div className="settings-section deployment-section"><div className="section-heading"><div><h2>{copy.deployment}</h2><p>Docker</p></div><Server size={21} className="section-icon" aria-hidden="true" /></div><p className="deployment-copy">{copy.deploymentHint}</p></div>
      </div>
      <div className="settings-footnote"><ShieldCheck size={15} aria-hidden="true" /> {copy.privacyFootnote}</div>
    </section>
  );
}

function NetworkInterfaceGroup({ title, hint, networks, mode, selected, labels, copy, disabled, collapsible = false, onToggle, onLabel }: { title: string; hint: string; networks: NetworkInterfaceInfo[]; mode: NetworkMode; selected: Set<string>; labels: Record<string, string>; copy: Record<string, string>; disabled: boolean; collapsible?: boolean; onToggle: (name: string) => void; onLabel: (name: string, label: string) => void }) {
  const rows = <div className="network-interface-grid">{networks.map((network) => <NetworkInterfaceRow key={network.name} network={network} mode={mode} selected={selected.has(network.name)} label={labels[network.name] ?? ''} copy={copy} disabled={disabled} onToggle={() => onToggle(network.name)} onLabel={(label) => onLabel(network.name, label)} />)}</div>;
  const heading = <div className="network-group-heading"><div><h3>{title}</h3><p>{hint}</p></div><span>{networks.length}</span></div>;
  if (collapsible) {
    return <details className="network-group collapsible" open={mode === 'selected' && networks.some((network) => selected.has(network.name)) ? true : undefined}><summary><ChevronRight size={16} aria-hidden="true" />{heading}</summary>{rows}</details>;
  }
  return <section className="network-group" aria-label={title}>{heading}{rows}</section>;
}

function NetworkInterfaceRow({ network, mode, selected, label, copy, disabled, onToggle, onLabel }: { network: NetworkInterfaceInfo; mode: NetworkMode; selected: boolean; label: string; copy: Record<string, string>; disabled: boolean; onToggle: () => void; onLabel: (label: string) => void }) {
  const checked = network.discoveryCapable && (mode === 'all' || selected);
  const controlDisabled = disabled || mode === 'all' || !network.discoveryCapable;
  const Icon = network.kind === 'wifi' ? Wifi : network.kind === 'bridge' || network.kind === 'virtual' ? Server : network.kind === 'tunnel' ? ShieldCheck : Monitor;
  const kindLabel = copy[`interfaceKind${network.kind[0].toUpperCase()}${network.kind.slice(1)}`];
  const capability = network.ipv4Discovery && network.ipv6Discovery ? copy.dualStackMulticast : network.ipv6Discovery ? copy.ipv6Multicast : network.ipv4Discovery ? copy.ipv4Multicast : network.pointToPoint ? copy.pointToPoint : copy.manualOnly;
  return (
    <div className={`network-interface-row ${checked ? 'selected' : ''} ${mode === 'all' ? 'automatic' : ''} ${!network.discoveryCapable ? 'manual-only' : ''}`} title={!network.discoveryCapable ? copy.manualOnlyHint : undefined}>
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
