import {useCallback,useEffect,useMemo,useRef,useState} from 'react';
import {Tabs,Tooltip} from 'radix-ui';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {listen} from '@tauri-apps/api/event';
import {ArrowLeft,Copy,FolderOpen,LoaderCircle,Minus,Settings,Square,X} from 'lucide-react';
import {api,clipTitle,defaultConfig,type Clip,type Snapshot,type ExportConfig,type MovieProject} from './api';
import {IconButton,SaveAction} from './components';
import {Editor} from './Editor';
import type {Selection} from './Timeline';
import {Inspector} from './Inspector';
import {SettingsDialog} from './Settings';
import {ClipLibraryGrid,libraryItems} from './ClipLibraryGrid';
import {MovieWorkspace} from './MovieWorkspace';
import {loadProject} from './projectState';
import {useAppUpdates} from './useAppUpdates';
import './style.css';

const window=getCurrentWindow();const quick=new URLSearchParams(location.search).has('quick');
function Titlebar({tab,busy,onTab,onSettings,updateAvailable}:{tab:'clips'|'movie';busy:boolean;onTab:(tab:'clips'|'movie')=>void;onSettings:()=>void;updateAvailable:boolean}){
 return <header className={`titlebar ${quick?'quick-titlebar':''}`}>{quick?<span className="quick-window-title">Быстрый клип</span>:<nav className="workspace-tabs" role="tablist" aria-label="Рабочее пространство"><button role="tab" disabled={busy} aria-selected={tab==='clips'} onClick={()=>onTab('clips')}>Клипы</button><button role="tab" disabled={busy} aria-selected={tab==='movie'} onClick={()=>onTab('movie')}>Монтаж</button><IconButton label="Настройки" hint={updateAvailable?"Доступно обновление":"Настройки"} className="icon-button settings-trigger" onClick={onSettings}><Settings size={16}/>{updateAvailable&&<span className="update-dot" aria-label="Доступно обновление"/>}</IconButton></nav>}<div className="titlebar-drag" data-tauri-drag-region/><div className="window-actions">{!quick&&<><button aria-label="Свернуть" onClick={()=>void window.minimize()}><Minus size={13}/></button><button aria-label="Развернуть" onClick={()=>void window.toggleMaximize()}><Square size={11}/></button></>}<button className="window-close" aria-label="Закрыть" onClick={()=>void window.close()}><X size={15}/></button></div></header>;
}
export function App(){
 const [state,setState]=useState<Snapshot>();const [clips,setClips]=useState<Clip[]>([]);const [selected,setSelected]=useState<Clip>();const [selection,setSelection]=useState<Selection>({start:0,end:0,cursor:0});const [config,setConfig]=useState<ExportConfig>();
 const [settingsOpen,setSettingsOpen]=useState(false);const [busy,setBusy]=useState(false);const [message,setMessage]=useState('');const [error,setError]=useState('');const [tab,setTab]=useState<'clips'|'movie'>('clips');const [movieVisited,setMovieVisited]=useState(false);const [view,setView]=useState<'library'|'editor'>('library');const [capturing,setCapturing]=useState(false);
 const updates=useAppUpdates(!quick);
 const [project,setProject]=useState<MovieProject>({name:'Новый мувик',segments:[]});const [projectKey,setProjectKey]=useState('');const projectRef=useRef({project,key:projectKey});projectRef.current={project,key:projectKey};
 const scrollPosition=useRef(0);const saving=useRef(false);const captureRequest=useRef(false);
 const [clipName,setClipName]=useState('');
 const drafts=useRef(new Map<string,{selection:Selection;config:ExportConfig;name:string}>());const current=useRef({selected,selection,config,clipName});current.current={selected,selection,config,clipName};
 const selectedPath=useRef<string|undefined>(undefined);const visibleError=error||state?.error||'';const hardware=!!state?.encoders.some(e=>!['Auto','Cpu'].includes(e));
 useEffect(()=>{if(!projectKey)return;const timer=globalThis.setTimeout(()=>{try{localStorage.setItem(projectKey,JSON.stringify(project));}catch{setError('Не удалось сохранить проект');}},350);return()=>globalThis.clearTimeout(timer);},[project,projectKey]);
 useEffect(()=>{const save=()=>{try{if(projectRef.current.key)localStorage.setItem(projectRef.current.key,JSON.stringify(projectRef.current.project));}catch{}};globalThis.addEventListener('beforeunload',save);return()=>{globalThis.removeEventListener('beforeunload',save);save();};},[]);
 const add=useCallback((clip:Clip)=>setClips(items=>[clip,...items.filter(item=>item.path!==clip.path)]),[]);
 const update=useCallback((clip:Clip)=>{setClips(items=>items.map(c=>c.path===clip.path?clip:c));setSelected(c=>c?.path===clip.path?clip:c);},[]);
 const viewed=useCallback((clip:Clip)=>{if(clip.viewed)return;void api.viewed(clip.path).then(saved=>{if(saved)update(saved);}).catch(e=>setError(String(e)));},[update]);
 const select=useCallback((clip:Clip,hardware=false)=>{const prev=current.current;if(prev.selected&&prev.config)drafts.current.set(prev.selected.path,{selection:prev.selection,config:prev.config,name:prev.clipName});const draft=drafts.current.get(clip.path);selectedPath.current=clip.path;setSelected(clip);setSelection(draft?.selection??{start:0,end:clip.duration,cursor:0});setConfig(draft?.config??defaultConfig(clip,hardware));setClipName(draft?.name??'');setView('editor');},[]);
 useEffect(()=>{
  let live=true;let receivedState=false;const off:Array<()=>void>=[];
  const register=<T,>(name:string,handler:(payload:T)=>void)=>listen<T>(name,event=>{if(live)handler(event.payload);}).then(fn=>live?off.push(fn):fn());
  const stateListener=register<Snapshot>('state',value=>{receivedState=true;setState(value);});
  if(!quick)void register<Clip>('clip-added',add);
  void register<Clip>('clip-updated',update);
  void register<string>('notice',setMessage);
  void stateListener.then(()=>{receivedState=false;return api.snapshot();}).then(value=>{
   if(!live)return;
   if(!receivedState)setState(value);
   if(!quick){const key=`rewind.movie.v1:${value.profile_id??'default'}`;setProject(loadProject(key));setProjectKey(key);}
  }).catch(e=>{if(live)setError(String(e));});
  if(!quick)void api.library().then(items=>{if(live)setClips(current=>[...current,...items.filter(item=>!current.some(c=>c.path===item.path))]);}).catch(e=>{if(live)setError(String(e));});
  if(!quick)void window.onDragDropEvent(event=>{if(event.payload.type==='drop')void api.importPaths(event.payload.paths).then(items=>{items.forEach(add);if(items[0])select(items[0]);}).catch(e=>setError(String(e)));}).then(fn=>live?off.push(fn):fn());
  return()=>{live=false;off.forEach(fn=>fn());};
 },[add,select,update]);
 const pending=state?.quick;
 useEffect(()=>{
  if(!quick||!pending)return;
  if(!pending.some(clip=>clip.path===selectedPath.current)){
   if(pending[0])select(pending[0],hardware);
   else if(selectedPath.current){selectedPath.current=undefined;setSelected(undefined);setConfig(undefined);}
  }
  for(const path of drafts.current.keys()){
   if(!pending.some(clip=>clip.path===path))drafts.current.delete(path);
  }
 },[pending,hardware,select]);
 useEffect(()=>{if(!message)return;const timer=globalThis.setTimeout(()=>setMessage(''),2600);return()=>globalThis.clearTimeout(timer);},[message]);
 useEffect(()=>{if(state?.replay_status!=='saving'){setCapturing(false);captureRequest.current=false;}},[state?.replay_status,state?.error]);
 useEffect(()=>{const key=(e:KeyboardEvent)=>{if(e.defaultPrevented)return;if(e.key!=='Escape'||document.fullscreenElement||settingsOpen||busy)return;if(quick)void window.close();else if(tab==='clips'&&view==='editor')setView('library');};globalThis.addEventListener('keydown',key);return()=>globalThis.removeEventListener('keydown',key);},[settingsOpen,busy,tab,view]);
 useEffect(()=>{if(hardware)setConfig(c=>c&&c.quality!=='Copy'&&c.quality!=='Target'&&c.encoding.format!=='Webm'&&c.encoding.format!=='Gif'?{...c,encoding:{...c.encoding,hardware:true}}:c);},[hardware,selected?.path]);
 const imported=()=>void api.import().then(clip=>{if(clip){add(clip);select(clip,hardware);}}).catch(e=>setError(String(e)));
 const capture=async()=>{if(!state||captureRequest.current)return;captureRequest.current=true;setCapturing(true);try{if(state.replay_status==='ready'||state.recording)await api.saveReplay();else await api.record(true);}catch(e){setError(String(e));setCapturing(false);captureRequest.current=false;}};
 const saveClip=useCallback(async(copy=false)=>{
  if(!selected||!config||saving.current||busy)return;
  saving.current=true;setBusy(true);
  try{
   const saved=await api.export(selected,selection.start,selection.end,structuredClone(config),quick?clipName.trim():undefined,copy);
   if(saved&&!quick){add(saved);select(saved,hardware);setMessage('Копия сохранена');}
  }catch(e){setError(String(e));}
  finally{saving.current=false;setBusy(false);}
 },[selected,config,busy,hardware,selection,clipName,add,select]);
 const openClip=(clip:Clip)=>{select(clip,hardware);viewed(clip);};
 const rootPath=selected?.original_path??selected?.path;
 const items=useMemo(()=>quick?pending?.map(clip=>({key:clip.path,clip,versions:[clip]}))??[]:view==='editor'?libraryItems(clips):[],[pending,view,clips]);
 const index=items.findIndex(item=>item.key===rootPath);
 const versions=items[index]?.versions??[];
 const original=versions.find(clip=>!clip.original_path);
 const edited=versions.filter(clip=>clip.original_path).reverse();
 const selectedTitle=selected?clipTitle(selected.title?selected:original??selected):'';
 const previous=items[index-1]?.clip;
 const next=index>=0?items[index+1]?.clip:undefined;
 const navigation=previous||next?{
  previous:previous?()=>openClip(previous):undefined,
  next:next?()=>openClip(next):undefined,
 }:undefined;
 const goTab=(next:'clips'|'movie')=>{setTab(next);if(next==='clips')setView('library');else setMovieVisited(true);};
 const action=<SaveAction busy={busy} disabled={!selected} onSave={()=>void saveClip()}/>;
 return <Tooltip.Provider><div className={`app ${quick?'quick-app':''}`}><Titlebar tab={tab} busy={busy} updateAvailable={!!updates.status?.available} onTab={goTab} onSettings={()=>setSettingsOpen(true)}/>
 {quick?<Tabs.Root className="quick-workspace" value={selected?.path??''} activationMode="manual" onValueChange={path=>{
  const clip=pending?.find(item=>item.path===path);
  if(clip&&clip.path!==selectedPath.current)select(clip,hardware);
 }}>
  {!!pending?.length&&<Tabs.List className="quick-tabs" aria-label="Быстрые клипы">
   {pending.map(clip=>{
    const title=clipTitle(clip);
    return <Tabs.Trigger key={clip.path} value={clip.path} title={title} disabled={busy}>{title}</Tabs.Trigger>;
   })}
  </Tabs.List>}
  {selected?<Tabs.Content value={selected.path} className="quick-body">
   <Editor clip={selected} playHotkey={state?.settings.editor_hotkeys?.play} audioTracks={config?.audioTracks} selection={selection} onChange={setSelection} compact busy={busy} navigation={navigation} onViewed={()=>viewed(selected)} onSave={()=>void saveClip()}/>
  </Tabs.Content>:<main className="quick-body"><div className="empty-state"><LoaderCircle size={20} className="spin"/></div></main>}
  <footer className="quick-footer">{busy?action:<div className="quick-save-row">
   <input aria-label="Название клипа" placeholder="Название клипа" maxLength={80} value={selected?clipName:''} disabled={!selected} onChange={e=>setClipName(e.target.value)} onKeyDown={e=>{if(e.key==='Enter'&&!e.nativeEvent.isComposing)void saveClip();}}/>
   <button className="secondary-button" aria-label="Скопировать в буфер обмена" disabled={!selected} onClick={()=>void saveClip(true)}><Copy size={15}/>Скопировать</button>
   {action}
  </div>}</footer>
 </Tabs.Root>:<>
 <div className="workspace" hidden={tab!=='clips'}>{view==='library'?<ClipLibraryGrid clips={clips} onOpen={openClip} onImport={imported} onCapture={()=>void capture()} onError={setError} state={capturing&&state?{...state,replay_status:'saving'}:state} scrollPosition={scrollPosition}/>:selected&&config&&state?<div className="main-layout">
  <main className="main-editor">
   <header className="editor-header">
    <IconButton label="Назад к клипам" disabled={busy} onClick={()=>setView('library')}><ArrowLeft size={17}/></IconButton>
    <span className="current-name" title={selectedTitle}>{selectedTitle}</span>
    <IconButton label="Показать в папке" onClick={()=>void api.revealClip(selected.path).catch(e=>setError(String(e)))}><FolderOpen size={16}/></IconButton>
    <span className="spacer"/>
    {!!edited.length&&<div className="version-switch" aria-label="Версия клипа">
     {original&&<button disabled={busy} className={original.path===selected.path?'active':''} aria-pressed={original.path===selected.path} onClick={()=>openClip(original)}>Исходник</button>}
     {edited.map((clip,i)=>{
      const title=clip.title??`V${i+1}`;
      return <button key={clip.path} disabled={busy} title={title} className={clip.path===selected.path?'active':''} aria-pressed={clip.path===selected.path} onClick={()=>openClip(clip)}>{title}</button>;
     })}
    </div>}
   </header>
   <Editor clip={selected} playHotkey={state.settings.editor_hotkeys?.play} audioTracks={config.audioTracks} selection={selection} onChange={setSelection} busy={busy} suspended={settingsOpen||tab!=='clips'} navigation={navigation} onViewed={()=>viewed(selected)}/>
  </main>
  <Inspector clip={selected} selection={selection} config={config} setConfig={setConfig} state={state} busy={busy}>{action}</Inspector>
 </div>:null}</div>
 {state&&movieVisited&&<div className="workspace" hidden={tab!=='movie'}><MovieWorkspace clips={clips} state={state} project={project} setProject={setProject} onImport={imported} onViewed={viewed} onSettings={()=>setSettingsOpen(true)} onError={setError} suspended={settingsOpen||tab!=='movie'} onBusy={setBusy} onSaved={clip=>{add(clip);setMessage('Видео сохранено');}}/></div>}
 </>}
 {state&&!quick&&<SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} state={state} onError={setError} updates={updates} exporting={busy} saveProject={()=>{if(projectKey)localStorage.setItem(projectKey,JSON.stringify(project));}}/>}{message&&!visibleError&&<div className="toast" role="status">{message}</div>}{visibleError&&<div className="toast error-toast" role="alert"><span>{visibleError}</span><button aria-label="Закрыть уведомление" onClick={()=>{setError('');setState(current=>current?{...current,error:null}:current);void api.dismiss();}}><X size={15}/></button></div>}
 </div></Tooltip.Provider>;
}
