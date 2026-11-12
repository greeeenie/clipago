import {useEffect,useState,type KeyboardEvent} from 'react';
import {Dialog,Tabs} from 'radix-ui';
import {ArrowUpRight,BookOpen,CircleDot,Clapperboard,Film,FolderOpen,Github,Info,Keyboard,RotateCcw,Settings2,X,type LucideIcon} from 'lucide-react';
import {Select,Slider,Switch} from './components';
import {api,snapBuffer,type Settings,type Snapshot,type Encoder,type EditorHotkeys} from './api';
import {editorActions,editorHotkeys,hotkeyLabel,keyCombination,movieActions,playbackAction} from './editorHotkeys';
import type {useAppUpdates} from './useAppUpdates';
import {version} from '../package.json';

const appIcon=new URL('../icons/clipago/128x128.png',import.meta.url).href;
const legacyReplayShortcuts:Record<string,string>={CtrlShiftS:'Ctrl+Shift+KeyS',AltF10:'Alt+F10',CtrlAltS:'Ctrl+Alt+KeyS'};
type Binding = 'shortcut' | keyof EditorHotkeys;
const encoderLabels:Record<Encoder,string>={Auto:'Автоматически',Nvidia:'NVIDIA NVENC',Intel:'Intel Quick Sync',Amd:'AMD AMF',Cpu:'CPU · x264'};
const sections=[
  {id:'general',title:'Общие',icon:Settings2},
  {id:'hotkeys',title:'Горячие клавиши',icon:Keyboard},
  {id:'about',title:'О программе',icon:Info},
] as const;

function SettingsHeading({title,icon:Icon}:{title:string;icon:LucideIcon}){
  return <div className="settings-heading"><h3><Icon size={18} aria-hidden="true"/>{title}</h3></div>;
}

export function SettingsDialog({open,onOpenChange,state,onError,updates,exporting,saveProject}:{open:boolean;onOpenChange:(v:boolean)=>void;state:Snapshot;onError:(message:string)=>void;updates:ReturnType<typeof useAppUpdates>;exporting:boolean;saveProject:()=>void}){
  const [draft,setDraft]=useState(state.settings);useEffect(()=>setDraft(state.settings),[state.settings]);
  const [section,setSection]=useState<(typeof sections)[number]>(sections[0]);
  const [resetting,setResetting]=useState(false);
  const [binding,setBinding]=useState<Binding>();const hotkeys=editorHotkeys(draft);
  const replayShortcut=legacyReplayShortcuts[draft.shortcut]??draft.shortcut;
  const save=(next:Settings)=>{setDraft(next);return api.settings(next).catch(e=>onError(String(e)));};
  const recordBinding=(id:Binding,e:KeyboardEvent<HTMLButtonElement>)=>{
    if(binding!==id)return;
    e.stopPropagation();
    if(e.key==='Tab'){setBinding(undefined);return;}
    e.preventDefault();
    if(e.key==='Escape'){setBinding(undefined);return;}
    const value=keyCombination(e);
    if(!value||e.repeat)return;
    const conflict=editorActions.find(action=>action.id!==id&&hotkeys[action.id]===value);
    if(conflict){onError(`Это сочетание уже назначено: ${conflict.label}`);return;}
    if(id!=='shortcut'&&value===replayShortcut){onError('Это сочетание уже назначено: Сохранить клип');return;}
    save(id==='shortcut'?{...draft,shortcut:value}:{...draft,editor_hotkeys:{...hotkeys,[id]:value}});
    setBinding(undefined);
  };
  const bindingButton=(id:Binding,labelId:string,value:string)=>(
    <button className={`hotkey-input ${binding===id?'recording':''}`} aria-labelledby={labelId}
      onClick={()=>setBinding(id)} onBlur={()=>setBinding(undefined)} onKeyDown={e=>recordBinding(id,e)}>
      {binding===id?'Нажми сочетание':<kbd>{hotkeyLabel(value)}</kbd>}
    </button>
  );
  const hotkeyRow=(id:Binding,label:string,value:string)=>(
    <div className="setting-row" key={id}><span id={`hotkey-${id}`}>{label}</span>{bindingButton(id,`hotkey-${id}`,value)}</div>
  );
  const restoreDefaults=async()=>{
    if(resetting||section.id==='about')return;
    setBinding(undefined);
    setResetting(true);
    try{
      const defaults=await api.defaultSettings();
      const {buffer_seconds,fps,monitor,encoder,output_dir,system_audio,microphone,shortcut,editor_hotkeys}=defaults;
      await save({...draft,...(section.id==='hotkeys'?{shortcut,editor_hotkeys}:{buffer_seconds,fps,monitor,encoder,output_dir,system_audio,microphone})});
    }catch(e){
      onError(String(e));
    }finally{
      setResetting(false);
    }
  };
  const updateMessage=updates.error||(updates.status?.available
    ? (updates.status.ui_only?'Обновится интерфейс. Запись продолжится.':'Для установки Clipago перезапустится.')
    : updates.status?'Установлена последняя версия':'');
  return <Dialog.Root open={open} onOpenChange={next=>{if(!next){setSection(sections[0]);setBinding(undefined);}onOpenChange(next);}}><Dialog.Portal><Dialog.Overlay className="dialog-backdrop settings-backdrop"/><Dialog.Content className="settings-dialog" aria-describedby={undefined} onEscapeKeyDown={event=>{if(binding){event.preventDefault();setBinding(undefined);}}}>
    <Tabs.Root className="settings-layout" value={section.id} orientation="vertical" onValueChange={id=>{
      const next=sections.find(item=>item.id===id);
      if(next)setSection(next);
      setBinding(undefined);
    }}>
      <aside className="settings-sidebar">
        <Dialog.Title>Настройки</Dialog.Title>
        <Tabs.List className="settings-nav" aria-label="Разделы настроек">
          {sections.map(({id,title,icon:Icon})=><Tabs.Trigger className="settings-tab" key={id} value={id}><Icon size={18} aria-hidden="true"/>{title}{id==='about'&&updates.status?.available&&<span className="settings-update-indicator" aria-label="Доступно обновление"/>}</Tabs.Trigger>)}
        </Tabs.List>
      </aside>
      <header className="settings-toolbar">
        {section.id!=='about'&&<span className="settings-toolbar-title">{section.title}</span>}
        <div className="settings-toolbar-actions">
          {section.id!=='about'&&<button type="button" className="settings-reset" disabled={resetting||(section.id==='general'&&state.transitioning)} onClick={()=>void restoreDefaults()}><RotateCcw size={14} aria-hidden="true"/>Вернуть по умолчанию</button>}
          <Dialog.Close asChild><button className="icon-button" aria-label="Закрыть настройки"><X size={17}/></button></Dialog.Close>
        </div>
      </header>
      <Tabs.Content className="settings-tab-panel" value="general">
        <div className="settings-stack">
          <section className="settings-section">
            <SettingsHeading title="Запись" icon={CircleDot}/>
            <div className="settings-card">
              <div className="setting-row"><span>Фоновая запись</span><Switch value={state.recording} label="Фоновая запись" disabled={state.transitioning} onChange={value=>api.record(value).catch(e=>onError(String(e)))}/></div>
              <div className="setting-row"><span>Системный звук</span><Switch value={draft.system_audio} label="Системный звук" onChange={system_audio=>save({...draft,system_audio})}/></div>
              <div className="setting-row"><span>Микрофон</span><Switch value={!!draft.microphone} label="Микрофон" onChange={microphone=>save({...draft,microphone})}/></div>
              <div className="setting-row"><span>Папка клипов</span><button className="folder-picker" aria-label="Выбрать папку клипов" title={draft.output_dir} onClick={()=>api.chooseFolder().then(path=>{if(path)save({...draft,output_dir:path});}).catch(e=>onError(String(e)))}><span>{draft.output_dir}</span><FolderOpen size={15} aria-hidden="true"/></button></div>
              <div className="buffer-setting">
                <div className="setting-heading"><span>Длина буфера</span><output>{draft.buffer_seconds} с</output></div>
                <Slider value={draft.buffer_seconds} min={30} max={120} step={5} label="Длина буфера" onChange={value=>setDraft({...draft,buffer_seconds:snapBuffer(value)})} onCommit={value=>save({...draft,buffer_seconds:snapBuffer(value)})}/>
                <div className="slider-labels"><span>30 с</span><span>2 мин</span></div>
              </div>
            </div>
          </section>
          <section className="settings-section">
            <SettingsHeading title="Видео" icon={Film}/>
            <div className="settings-card">
              <div className="setting-row"><span>FPS записи</span><div className="segments settings-fps" role="group" aria-label="FPS записи">{[30,60].map(fps=><button key={fps} className={draft.fps===fps?'active':''} aria-pressed={draft.fps===fps} disabled={state.transitioning} onClick={()=>save({...draft,fps})}>{fps} FPS</button>)}</div></div>
              <div className="setting-row"><span>Экран</span><Select value={String(draft.monitor)} label="Экран записи" disabled={state.transitioning} onChange={value=>save({...draft,monitor:Number(value)})} options={state.monitors.map(m=>({value:String(m.index),label:m.name}))}/></div>
              <div className="setting-row"><span>Кодировщик</span><Select value={draft.encoder} label="Кодировщик записи" disabled={state.transitioning} onChange={encoder=>save({...draft,encoder})} options={[...new Set([...state.encoders,draft.encoder])].map(value=>({value,label:encoderLabels[value]}))}/></div>
            </div>
          </section>
        </div>
      </Tabs.Content>
      <Tabs.Content className="settings-tab-panel" value="hotkeys">
        <div className="settings-stack">
          <section className="settings-section">
            <SettingsHeading title="Общие" icon={Keyboard}/>
            <div className="settings-card">
              {hotkeyRow('shortcut','Сохранить клип',replayShortcut)}
              {hotkeyRow(playbackAction.id,playbackAction.label,hotkeys.play)}
            </div>
          </section>
          <section className="settings-section">
            <SettingsHeading title="Монтаж" icon={Clapperboard}/>
            <div className="settings-card">
              {movieActions.map(action=>hotkeyRow(action.id,action.label,hotkeys[action.id]))}
            </div>
          </section>
        </div>
      </Tabs.Content>
      <Tabs.Content className="settings-tab-panel" value="about">
        <div className="settings-stack">
          <div className="settings-brand"><img src={appIcon} alt=""/><div><strong>Clipago</strong><span>Версия {updates.status?.current??version}</span></div></div>
          <div className="settings-card settings-update">
            <div className="settings-update-info"><strong>Обновления</strong>{updateMessage&&<p className="update-message" role="status">{updateMessage}</p>}</div>
            <div className="settings-update-actions">
              {updates.status?.available
                ? <button className="primary-button" disabled={exporting||!!updates.activity} onClick={()=>void updates.install(saveProject)}>{updates.activity==='installing'?'Установка…':`Установить ${updates.status.available}`}</button>
                : <button className="secondary-button" disabled={!!updates.activity} onClick={()=>void updates.check()}>{updates.activity==='checking'?'Проверка…':'Проверить'}</button>}
            </div>
          </div>
          <div className="settings-card">
            <button className="settings-link" onClick={()=>void api.openHelp('github').catch(e=>onError(String(e)))}><Github size={16} aria-hidden="true"/>GitHub<ArrowUpRight size={16} aria-hidden="true"/></button>
            <button className="settings-link" onClick={()=>void api.openHelp('docs').catch(e=>onError(String(e)))}><BookOpen size={16} aria-hidden="true"/>Документация<ArrowUpRight size={16} aria-hidden="true"/></button>
          </div>
        </div>
      </Tabs.Content>
    </Tabs.Root>
  </Dialog.Content></Dialog.Portal></Dialog.Root>;
}
