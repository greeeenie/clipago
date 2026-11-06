import {useEffect,useState} from 'react';
import {Dialog,Collapsible} from 'radix-ui';
import {ChevronDown,FolderOpen,RotateCcw,X} from 'lucide-react';
import {IconButton,Select,Slider,Switch} from './components';
import {api,snapBuffer,type Settings,type Snapshot,type Encoder} from './api';
import {defaultEditorHotkeys,editorActions,editorHotkeys,hotkeyLabel,keyCombination} from './editorHotkeys';
import type {useAppUpdates} from './useAppUpdates';
const encoderLabels:Record<Encoder,string>={Auto:'Автоматически',Nvidia:'NVIDIA NVENC',Intel:'Intel Quick Sync',Amd:'AMD AMF',Cpu:'CPU · x264'};
export function SettingsDialog({open,onOpenChange,state,onError,updates,exporting,saveProject}:{open:boolean;onOpenChange:(v:boolean)=>void;state:Snapshot;onError:(message:string)=>void;updates:ReturnType<typeof useAppUpdates>;exporting:boolean;saveProject:()=>void}){
  const [draft,setDraft]=useState(state.settings);useEffect(()=>setDraft(state.settings),[state.settings]);
  const [binding,setBinding]=useState<string>();const hotkeys=editorHotkeys(draft);
  const save=(next:Settings)=>{setDraft(next);api.settings(next).catch(e=>onError(String(e)));};
  return <Dialog.Root open={open} onOpenChange={onOpenChange}><Dialog.Portal><Dialog.Overlay className="dialog-backdrop"/><Dialog.Content className="settings-dialog" aria-describedby={undefined}>
    <header className="dialog-header"><Dialog.Title>Настройки</Dialog.Title><Dialog.Close asChild><button className="icon-button" aria-label="Закрыть настройки"><X size={17}/></button></Dialog.Close></header>
    <div className="settings-scroll">
      <div className="setting-row recording-row"><label htmlFor="record-toggle">Фоновая запись</label><Switch value={state.recording} label="Фоновая запись" disabled={state.transitioning} onChange={value=>api.record(value).catch(e=>onError(String(e)))}/></div>
      <div className="setting-row"><label>Системный звук</label><Switch value={draft.system_audio} label="Системный звук" onChange={system_audio=>save({...draft,system_audio})}/></div>
      <div className="buffer-setting"><div className="setting-heading"><label>Длина буфера</label><output>{draft.buffer_seconds} с</output></div>
        <Slider value={draft.buffer_seconds} min={30} max={120} step={5} label="Длина буфера" onChange={value=>setDraft({...draft,buffer_seconds:snapBuffer(value)})} onCommit={value=>save({...draft,buffer_seconds:snapBuffer(value)})}/>
        <div className="slider-labels"><span>30 с</span><span>2 мин</span></div>
      </div>
      <div className="settings-divider"/>
      <div className="setting-row"><label>FPS записи</label><div className="segments">{[30,60].map(fps=><button key={fps} className={draft.fps===fps?'active':''} disabled={state.transitioning} onClick={()=>save({...draft,fps})}>{fps} FPS</button>)}</div></div>
      <Collapsible.Root className="advanced hotkey-settings"><Collapsible.Trigger className="disclosure">Горячие клавиши<ChevronDown size={14}/></Collapsible.Trigger><Collapsible.Content className="advanced-content">
        <div className="hotkey-reset"><IconButton label="Сбросить горячие клавиши" onClick={()=>{setBinding(undefined);save({...draft,editor_hotkeys:{...defaultEditorHotkeys}});}}><RotateCcw size={15}/></IconButton></div>
        {editorActions.map(action=><div className="setting-row" key={action.id}><label id={`hotkey-${action.id}`}>{action.label}</label><button className={`hotkey-input ${binding===action.id?'recording':''}`} aria-labelledby={`hotkey-${action.id}`} onClick={()=>setBinding(action.id)} onBlur={()=>setBinding(undefined)} onKeyDown={e=>{
          if(binding!==action.id)return;e.stopPropagation();if(e.key==='Tab'){setBinding(undefined);return;}e.preventDefault();if(e.key==='Escape'){setBinding(undefined);return;}const value=keyCombination(e);if(!value)return;
          const conflict=editorActions.find(other=>other.id!==action.id&&hotkeys[other.id]===value);if(conflict){onError(`Это сочетание уже назначено: ${conflict.label}`);return;}
          save({...draft,editor_hotkeys:{...hotkeys,[action.id]:value}});setBinding(undefined);
        }}>{binding===action.id?'Нажми сочетание':<kbd>{hotkeyLabel(hotkeys[action.id])}</kbd>}</button></div>)}
      </Collapsible.Content></Collapsible.Root>
      <Collapsible.Root className="advanced"><Collapsible.Trigger className="disclosure">Дополнительно<ChevronDown size={14}/></Collapsible.Trigger><Collapsible.Content className="advanced-content">
      <div className="setting-row"><label>Экран</label><Select value={String(draft.monitor)} label="Экран записи" disabled={state.transitioning} onChange={value=>save({...draft,monitor:Number(value)})} options={state.monitors.map(m=>({value:String(m.index),label:m.name}))}/></div>
      <div className="setting-row"><label>Кодировщик</label><Select value={draft.encoder} label="Кодировщик записи" disabled={state.transitioning} onChange={encoder=>save({...draft,encoder})} options={[...new Set([...state.encoders,draft.encoder])].map(value=>({value,label:encoderLabels[value]}))}/></div>
      <div className="setting-row folder-row"><label>Папка клипов</label><button className="folder-picker" title={draft.output_dir} onClick={()=>api.chooseFolder().then(path=>{if(path)save({...draft,output_dir:path});}).catch(e=>onError(String(e)))}><span>{draft.output_dir}</span><FolderOpen size={15}/></button></div>
      <div className="settings-divider"/>
      <div className="setting-row"><label>Сохранить клип</label><Select value={draft.shortcut} label="Горячая клавиша клипа" onChange={shortcut=>save({...draft,shortcut})} options={[{value:'CtrlShiftS',label:'Ctrl + Shift + S'},{value:'AltF10',label:'Alt + F10'},{value:'CtrlAltS',label:'Ctrl + Alt + S'}]}/></div>
        {(['ffmpeg','ffprobe'] as const).map(key=><div className="setting-row" key={key}><label htmlFor={key}>{key==='ffmpeg'?'FFmpeg':'FFprobe'}</label><input id={key} value={draft[key]} onChange={e=>setDraft({...draft,[key]:e.target.value})} onBlur={()=>{if(draft[key]!==state.settings[key])save(draft);}} onKeyDown={e=>{if(e.key==='Enter')e.currentTarget.blur();}} spellCheck={false}/></div>)}
      </Collapsible.Content></Collapsible.Root>
      <div className="settings-divider"/>
      <div className="setting-row update-row">
        <div>
          <div>Обновления</div>
          {updates.status&&<small className="update-version">Clipago {updates.status.current}</small>}
        </div>
        {updates.status?.available&&<button className="primary-button" disabled={exporting||!!updates.activity} onClick={()=>void updates.install(saveProject)}>
          {updates.activity==='installing'?'Установка…':`Установить ${updates.status.available}`}
        </button>}
        <button className="secondary-button" disabled={!!updates.activity} onClick={()=>void updates.check()}>
          {updates.activity==='checking'?'Проверка…':'Проверить'}
        </button>
      </div>
      {updates.error&&<p className="update-message" role="status">{updates.error}</p>}
      {updates.status&&!updates.status.available&&!updates.error&&<p className="update-message" role="status">Установлена последняя версия</p>}
    </div>
  </Dialog.Content></Dialog.Portal></Dialog.Root>;
}
