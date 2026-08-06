import { useEffect, useId, useRef } from 'react';
import type { ReactNode } from 'react';

export type ConfirmDialogTone = 'default' | 'warning' | 'danger';

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  cancelLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
  icon?: ReactNode;
  tone?: ConfirmDialogTone;
  dismissOnBackdrop?: boolean;
}

export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel,
  onConfirm,
  onCancel,
  icon,
  tone = 'default',
  dismissOnBackdrop = true,
}: ConfirmDialogProps) {
  const dialogRef = useRef<HTMLElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const onCancelRef = useRef(onCancel);
  const titleId = useId();
  const descriptionId = useId();

  useEffect(() => {
    onCancelRef.current = onCancel;
  }, [onCancel]);

  useEffect(() => {
    if (!open) return;
    returnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusTimer = window.requestAnimationFrame(() => {
      dialogRef.current?.querySelector<HTMLButtonElement>('[data-confirm-dialog-cancel]')?.focus();
    });
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCancelRef.current();
        return;
      }
      if (event.key !== 'Tab') return;
      const buttons = [...(dialogRef.current?.querySelectorAll<HTMLButtonElement>('button:not([disabled])') ?? [])];
      const first = buttons[0];
      const last = buttons[buttons.length - 1];
      if (!first || !last) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      window.cancelAnimationFrame(focusTimer);
      document.removeEventListener('keydown', onKeyDown);
      returnFocusRef.current?.focus();
    };
  }, [open]);

  if (!open) return null;
  return (
    <div className="confirm-dialog-backdrop" onMouseDown={(event) => { if (dismissOnBackdrop && event.target === event.currentTarget) onCancel(); }}>
      <section ref={dialogRef} className="confirm-dialog" data-tone={tone} role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId}>
        {icon ? <span className="confirm-dialog-icon" data-tone={tone}>{icon}</span> : null}
        <h2 id={titleId}>{title}</h2>
        <p id={descriptionId}>{description}</p>
        <div className="confirm-dialog-actions">
          <button className="secondary-button" type="button" data-confirm-dialog-cancel onClick={onCancel}>{cancelLabel}</button>
          <button className={`primary-button${tone === 'danger' ? ' danger-button' : ''}`} type="button" onClick={onConfirm}>{confirmLabel}</button>
        </div>
      </section>
    </div>
  );
}
