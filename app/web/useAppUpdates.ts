import {useCallback, useEffect, useRef, useState} from 'react';
import {invoke} from '@tauri-apps/api/core';

type UpdateStatus = {current: string; available: string | null; ui_only: boolean};

export function useAppUpdates(enabled: boolean) {
  const [status, setStatus] = useState<UpdateStatus>();
  const [activity, setActivity] = useState<'checking' | 'installing'>();
  const [error, setError] = useState('');
  const pending = useRef(false);

  const check = useCallback(async (manual = true) => {
    if (pending.current) return;
    pending.current = true;
    if (manual) {
      setActivity('checking');
      setError('');
    }
    try {
      setStatus(await invoke<UpdateStatus>('check_update'));
      setError('');
    } catch {
      if (manual) setError('Не удалось проверить обновления. Попробуйте ещё раз.');
    } finally {
      pending.current = false;
      setActivity(undefined);
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    const firstCheck = globalThis.setTimeout(() => void check(false), 1500);
    const periodicCheck = globalThis.setInterval(() => void check(false), 6 * 60 * 60 * 1000);
    return () => {
      globalThis.clearTimeout(firstCheck);
      globalThis.clearInterval(periodicCheck);
    };
  }, [check, enabled]);

  async function install(saveProject: () => void) {
    const version = status?.available;
    if (!version || pending.current) return;
    pending.current = true;
    setActivity('installing');
    setError('');
    try {
      saveProject();
      await invoke('install_update', {version, uiOnly: status.ui_only});
    } catch (reason) {
      const details = String(reason);
      setError(details.startsWith('Релиз изменился') || details.startsWith('Дождитесь')
        ? details : 'Не удалось установить обновление. Проверьте обновления ещё раз.');
      setStatus(undefined);
    } finally {
      pending.current = false;
      setActivity(undefined);
    }
  }

  return {status, activity, error, check, install};
}
