import { useEffect, useId, useRef, useState } from 'react';
import { X } from 'lucide-react';
import QRCode from 'qrcode';

interface QrDialogProps {
  open: boolean;
  value: string;
  label: string;
  closeLabel: string;
  onClose: () => void;
}

export function QrDialog({ open, value, label, closeLabel, onClose }: QrDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [imageUrl, setImageUrl] = useState('');
  const titleId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    void QRCode.toDataURL(value, {
      width: 320,
      margin: 2,
      errorCorrectionLevel: 'M',
      color: { dark: '#0f172a', light: '#ffffff' },
    }).then((url) => {
      if (active) setImageUrl(url);
    });
    return () => { active = false; };
  }, [open, value]);

  return (
    <dialog
      ref={dialogRef}
      className="qr-dialog"
      aria-labelledby={titleId}
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onClose={onClose}
      onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}
    >
      <div className="qr-dialog-content">
        <div className="qr-dialog-header">
          <h2 id={titleId}>{label}</h2>
          <button className="icon-button" type="button" title={closeLabel} aria-label={closeLabel} onClick={onClose}>
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <div className="qr-code-frame">
          {imageUrl ? <img src={imageUrl} width="320" height="320" alt={label} /> : null}
        </div>
        <code>{label}</code>
      </div>
    </dialog>
  );
}
