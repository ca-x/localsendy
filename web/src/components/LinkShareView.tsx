import { useRef, useState } from 'react';
import {
  ArrowLeft,
  Check,
  CheckCircle2,
  CircleAlert,
  Copy,
  File,
  Link2,
  Monitor,
  QrCode,
  RefreshCw,
  Shield,
  X,
} from 'lucide-react';
import {
  decideLinkShareRequest,
  getLinkShare,
  updateLinkShare,
} from '../api';
import { formatBytes, formatTime } from '../format';
import type { LinkShare } from '../types';
import { ConfirmDialog } from './ConfirmDialog';
import { QrDialog } from './QrDialog';

interface LinkShareViewProps {
  copy: Record<string, string>;
  share: LinkShare;
  onShare: (share: LinkShare) => void;
  onBack: () => Promise<void>;
  onError: (message: string | null) => void;
  onNotice: (message: string | null) => void;
}

export function LinkShareView({ copy, share, onShare, onBack, onError, onNotice }: LinkShareViewProps) {
  const pinDialogRef = useRef<HTMLDialogElement>(null);
  const [isStopConfirmOpen, setIsStopConfirmOpen] = useState(false);
  const [draftPin, setDraftPin] = useState(share.pin ?? '');
  const [qrUrl, setQrUrl] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const pendingCount = share.requests.filter((request) => request.status === 'pending').length;

  async function copyUrl(url: string) {
    try {
      await navigator.clipboard.writeText(url);
      onNotice(copy.copiedToClipboard);
    } catch {
      onError(copy.clipboardUnavailable);
    }
  }

  async function saveSettings(autoAccept: boolean, pin: string) {
    setIsSaving(true);
    onError(null);
    try {
      onShare(await updateLinkShare(autoAccept, pin));
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    } finally {
      setIsSaving(false);
    }
  }

  async function decide(sessionId: string, decision: 'accept' | 'reject') {
    try {
      await decideLinkShareRequest(sessionId, decision);
      onShare(await getLinkShare());
    } catch (requestError) {
      onError(requestError instanceof Error ? requestError.message : copy.error);
    }
  }

  function openPinDialog() {
    setDraftPin(share.pin ?? String(Math.floor(100000 + Math.random() * 900000)));
    pinDialogRef.current?.showModal();
  }

  function closePinDialog() {
    pinDialogRef.current?.close();
  }

  function enablePin() {
    if (!draftPin.trim()) return;
    closePinDialog();
    void saveSettings(share.autoAccept, draftPin);
  }

  const firstUrl = share.urls[0] ?? `${window.location.origin}/share`;
  return (
    <section className="workspace link-share-workspace">
      <div className="link-share-topbar">
        <button className="icon-button outlined" type="button" title={copy.back} aria-label={copy.back} onClick={() => setIsStopConfirmOpen(true)}>
          <ArrowLeft size={18} aria-hidden="true" />
        </button>
        <div><h1>{copy.shareViaLink}</h1><p>{copy.linkShareActive}</p></div>
        <span className="share-live-badge"><span className="status-dot" aria-hidden="true" />{copy.active}</span>
      </div>

      <div className="link-share-grid">
        <div className="link-share-main">
          <section className="link-share-section">
            <div className="section-heading"><div><h2>{share.urls.length === 1 ? copy.openThisLink : copy.openOneLink}</h2><p>{copy.linkShareBrowserHint}</p></div></div>
            <div className="share-url-list">
              {share.urls.length > 0 ? share.urls.map((url) => (
                <div className="share-url-row" key={url}>
                  <code>{url}</code>
                  <button className="icon-button" type="button" title={copy.copyLink} aria-label={`${copy.copyLink}: ${url}`} onClick={() => void copyUrl(url)}><Copy size={17} aria-hidden="true" /></button>
                  <button className="icon-button" type="button" title={copy.showQrCode} aria-label={`${copy.showQrCode}: ${url}`} onClick={() => setQrUrl(url)}><QrCode size={18} aria-hidden="true" /></button>
                </div>
              )) : <div className="compact-empty"><CircleAlert size={18} aria-hidden="true" />{copy.noShareAddress}</div>}
            </div>
          </section>

          <section className="link-share-section requests-section">
            <div className="section-heading"><div><h2>{copy.requests}{pendingCount > 0 ? <span className="count-badge warning">{pendingCount}</span> : null}</h2><p>{copy.requestsHint}</p></div></div>
            {share.requests.length === 0 ? (
              <div className="share-empty"><Link2 size={20} aria-hidden="true" /><span>{copy.noRequests}</span></div>
            ) : (
              <div className="share-request-list">
                {share.requests.map((request) => (
                  <article className="share-request" key={request.sessionId}>
                    <span className="request-icon"><Monitor size={18} aria-hidden="true" /></span>
                    <span className="request-copy"><strong>{request.userAgent || copy.browser}</strong><small>{request.ip} · {formatTime(request.createdAt)}</small></span>
                    {request.status === 'pending' ? (
                      <span className="request-actions">
                        <button className="icon-button danger-outline" type="button" title={copy.reject} aria-label={`${copy.reject}: ${request.ip}`} onClick={() => void decide(request.sessionId, 'reject')}><X size={17} aria-hidden="true" /></button>
                        <button className="icon-button accept-button" type="button" title={copy.accept} aria-label={`${copy.accept}: ${request.ip}`} onClick={() => void decide(request.sessionId, 'accept')}><Check size={17} aria-hidden="true" /></button>
                      </span>
                    ) : <span className="accepted-label"><CheckCircle2 size={15} aria-hidden="true" />{copy.accepted}</span>}
                  </article>
                ))}
              </div>
            )}
          </section>
        </div>

        <aside className="link-share-side">
          <section className="link-share-section">
            <div className="section-heading"><div><h2>{copy.sharedFiles}<span className="count-badge">{share.files.length}</span></h2><p>{formatBytes(share.totalBytes)}</p></div></div>
            <div className="share-file-list">{share.files.map((file) => <div className="share-file" key={file.id}><span className="file-type-icon"><File size={16} aria-hidden="true" /></span><span><strong title={file.name}>{file.name}</strong><small>{formatBytes(file.size)}</small></span></div>)}</div>
          </section>

          <section className="link-share-section share-controls">
            <div className="section-heading"><div><h2>{copy.accessControl}</h2><p>{copy.accessControlHint}</p></div><Shield size={18} aria-hidden="true" /></div>
            <label className="share-setting-row"><span><strong>{copy.automaticallyAcceptRequests}</strong><small>{copy.autoAcceptLinkHint}</small></span><input className="sr-only" type="checkbox" checked={share.autoAccept} disabled={isSaving} onChange={(event) => void saveSettings(event.target.checked, share.pin ?? '')} /><span className="switch-track" aria-hidden="true"><span className="switch-thumb" /></span></label>
            <div className="share-setting-row pin-setting"><span><strong>{copy.requirePin}</strong><small>{share.pin ? copy.pinHint.replace('{pin}', share.pin) : copy.pinDisabledHint}</small></span><button className="secondary-button" type="button" disabled={isSaving} onClick={openPinDialog}>{share.pin ? copy.change : copy.enable}</button></div>
            {share.pin ? <button className="text-button danger-text" type="button" disabled={isSaving} onClick={() => void saveSettings(share.autoAccept, '')}>{copy.disablePin}</button> : null}
            {isSaving ? <div className="saving-setting" role="status"><RefreshCw size={15} className="spin" aria-hidden="true" />{copy.saving}</div> : null}
          </section>

          <button className="secondary-button stop-share-button" type="button" onClick={() => setIsStopConfirmOpen(true)}><X size={17} aria-hidden="true" />{copy.stopSharing}</button>
        </aside>
      </div>

      <ConfirmDialog open={isStopConfirmOpen} title={copy.stopSharingTitle} description={copy.stopSharingConfirm} confirmLabel={copy.stopSharing} cancelLabel={copy.cancel} icon={<CircleAlert size={20} aria-hidden="true" />} tone="danger" onCancel={() => setIsStopConfirmOpen(false)} onConfirm={() => { setIsStopConfirmOpen(false); void onBack(); }} />

      <dialog ref={pinDialogRef} className="pin-dialog" onCancel={(event) => { event.preventDefault(); closePinDialog(); }} onClick={(event) => { if (event.target === event.currentTarget) closePinDialog(); }}>
        <form className="pin-dialog-content" method="dialog" onSubmit={(event) => { event.preventDefault(); enablePin(); }}>
          <span className="confirm-dialog-icon"><Shield size={20} aria-hidden="true" /></span>
          <h2>{copy.requirePin}</h2>
          <p>{copy.pinChangeWarning}</p>
          <label htmlFor="link-share-pin">{copy.pin}</label>
          <input id="link-share-pin" autoFocus type="text" inputMode="numeric" autoComplete="off" maxLength={128} value={draftPin} onChange={(event) => setDraftPin(event.target.value)} />
          <div className="confirm-dialog-actions"><button className="secondary-button" type="button" onClick={closePinDialog}>{copy.cancel}</button><button className="primary-button" type="submit" disabled={!draftPin.trim()}>{copy.enablePin}</button></div>
        </form>
      </dialog>

      <QrDialog open={qrUrl !== null} value={share.pin ? `${qrUrl}?pin=${encodeURIComponent(share.pin)}` : qrUrl ?? firstUrl} label={qrUrl ?? firstUrl} closeLabel={copy.close} onClose={() => setQrUrl(null)} />
    </section>
  );
}
