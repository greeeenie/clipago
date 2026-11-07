import {useCallback,useEffect,useRef,useState} from 'react';
import {Tabs,Tooltip} from 'radix-ui';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {listen} from '@tauri-apps/api/event';
import {ArrowLeft,LoaderCircle,Minus,Settings,Square,X} from 'lucide-react';
import {api,clipTitle,defaultConfig,type Clip,type Snapshot,type ExportConfig,type MovieProject} from './api';
import {IconButton,SaveAction} from './components';
import {Editor} from './Editor';
import type {Selection} from './Timeline';
import {Inspector} from './Inspector';
import {SettingsDialog} from './Settings';
import {ClipLibraryGrid} from './ClipLibraryGrid';
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
 const drafts=useRef(new Map<string,{selection:Selection;config:ExportConfig}>());const current=useRef({selected,selection,config});current.current={selected,selection,config};
 const selectedPath=useRef<string|undefined>(undefined);const visibleError=error||state?.error||'';const hardware=!!state?.encoders.some(e=>!['Auto','Cpu'].includes(e));
 useEffect(()=>{if(!projectKey)return;const timer=globalThis.setTimeout(()=>{try{localStorage.setItem(projectKey,JSON.stringify(project));}catch{setError('Не удалось сохранить проект');}},350);return()=>globalThis.clearTimeout(timer);},[project,projectKey]);
 useEffect(()=>{const save=()=>{try{if(projectRef.current.key)localStorage.setItem(projectRef.current.key,JSON.stringify(projectRef.current.project));}catch{}};globalThis.addEventListener('beforeunload',save);return()=>{globalThis.removeEventListener('beforeunload',save);save();};},[]);
 const add=useCallback((clip:Clip)=>setClips(items=>[clip,...items.filter(item=>item.path!==clip.path)]),[]);
 const update=useCallback((clip:Clip)=>{setClips(items=>items.map(c=>c.path===clip.path?clip:c));setSelected(c=>c?.path===clip.path?clip:c);},[]);
 const viewed=useCallback((clip:Clip)=>{if(clip.viewed)return;void api.viewed(clip.path).then(saved=>{if(saved)update(saved);}).catch(e=>setError(String(e)));},[update]);
 const select=useCallback((clip:Clip,hardware=false)=>{const prev=current.current;if(prev.selected&&prev.config)drafts.current.set(prev.selected.path,{selection:prev.selection,config:prev.config});const draft=drafts.current.get(clip.path);selectedPath.current=clip.path;setSelected(clip);setSelection(draft?.selection??{start:0,end:clip.duration,cursor:0});setConfig(draft?.config??defaultConfig(clip,hardware));setView('editor');},[]);
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
 useEffect(()=>{const key=(e:KeyboardEvent)=>{if(e.defaultPrevented)return;if(e.key!=='Escape'||settingsOpen||busy)return;if(quick)void window.close();else if(tab==='clips'&&view==='editor')setView('library');};globalThis.addEventListener('keydown',key);return()=>globalThis.removeEventListener('keydown',key);},[settingsOpen,busy,tab,view]);
 useEffect(()=>{if(hardware)setConfig(c=>c&&c.quality!=='Copy'&&c.quality!=='Target'&&c.encoding.format!=='Webm'&&c.encoding.format!=='Gif'?{...c,encoding:{...c.encoding,hardware:true}}:c);},[hardware,selected?.path]);
 const imported=()=>void api.import().then(clip=>{if(clip){add(clip);select(clip,hardware);}}).catch(e=>setError(String(e)));
 const capture=async()=>{if(!state||captureRequest.current)return;captureRequest.current=true;setCapturing(true);try{if(state.replay_status==='ready'||state.recording)await api.saveReplay();else await api.record(true);}catch(e){setError(String(e));setCapturing(false);captureRequest.current=false;}};
 const saveClip=useCallback(async()=>{
  if(!selected||!config||saving.current||busy)return;
  saving.current=true;setBusy(true);
  try{
   const saved=await api.export(selected,selection.start,selection.end,structuredClone(config));
   if(saved&&!quick){add(saved);select(saved,hardware);setMessage('Копия сохранена');}
  }catch(e){setError(String(e));}
  finally{saving.current=false;setBusy(false);}
 },[selected,config,busy,hardware,selection,add,select]);
 const versions=selected?clips.filter(c=>c.path===selected.path||c.path===(selected.original_path??selected.path)||c.original_path===(selected.original_path??selected.path)):[];
 const selectedTitle=selected?clipTitle(versions.find(clip=>clip.path===selected.original_path)??selected):'';
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
  {selected?<Tabs.Content key={selected.path} value={selected.path} className="quick-body">
   <Editor clip={selected} audioTracks={config?.audioTracks} selection={selection} onChange={setSelection} compact busy={busy} onViewed={()=>viewed(selected)} onSave={()=>void saveClip()}/>
  </Tabs.Content>:<main className="quick-body"><div className="empty-state"><LoaderCircle size={20} className="spin"/></div></main>}
  <footer className="quick-footer">{action}</footer>
 </Tabs.Root>:<>
 <div className="workspace" hidden={tab!=='clips'}>{view==='library'?<ClipLibraryGrid clips={clips} onOpen={clip=>{select(clip,hardware);viewed(clip);}} onImport={imported} onCapture={()=>void capture()} onError={setError} state={capturing&&state?{...state,replay_status:'saving'}:state} scrollPosition={scrollPosition}/>:selected&&config&&state?<div className="main-layout"><main className="main-editor"><header className="editor-header"><IconButton label="Назад к клипам" disabled={busy} onClick={()=>setView('library')}><ArrowLeft size={17}/></IconButton><span className="current-name" title={selectedTitle}>{selectedTitle}</span><span className="spacer"/>{versions.length>1&&<div className="version-switch" aria-label="Версия клипа">{versions.map(c=><button key={c.path} disabled={busy} className={c.path===selected.path?'active':''} onClick={()=>{select(c,hardware);viewed(c);}}>{c.original_path?'Обработан':'Исходник'}</button>)}</div>}</header><Editor key={selected.path} clip={selected} audioTracks={config.audioTracks} selection={selection} onChange={setSelection} busy={busy} suspended={settingsOpen||tab!=='clips'} onViewed={()=>viewed(selected)}/></main><Inspector clip={selected} selection={selection} config={config} setConfig={setConfig} state={state} busy={busy}>{action}</Inspector></div>:null}</div>
 {state&&movieVisited&&<div className="workspace" hidden={tab!=='movie'}><MovieWorkspace clips={clips} state={state} project={project} setProject={setProject} onImport={imported} onViewed={viewed} onSettings={()=>setSettingsOpen(true)} onError={setError} suspended={settingsOpen||tab!=='movie'} onBusy={setBusy} onSaved={clip=>{add(clip);setMessage('Видео сохранено');}}/></div>}
 </>}
 {state&&!quick&&<SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} state={state} onError={setError} updates={updates} exporting={busy} saveProject={()=>{if(projectKey)localStorage.setItem(projectKey,JSON.stringify(project));}}/>}{message&&!visibleError&&<div className="toast" role="status">{message}</div>}{visibleError&&<div className="toast error-toast" role="alert"><span>{visibleError}</span><button aria-label="Закрыть уведомление" onClick={()=>{setError('');setState(current=>current?{...current,error:null}:current);void api.dismiss();}}><X size={15}/></button></div>}
 </div></Tooltip.Provider>;
}
