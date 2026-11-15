import {useLayoutEffect,useRef,useState,type RefObject} from 'react';
import {Menu,MenuContextTrigger,MenuContent,MenuItem} from '@ui-sdk/react/menu';
import {Dialog,DialogBackdrop,DialogContent,DialogTitle,DialogClose} from '@ui-sdk/react/dialog';
import {Check,Copy,Film,FolderOpen,Layers2,LoaderCircle,Play,Plus,Trash2,Video,X,Zap} from 'lucide-react';
import {api,clipTitle,time,type Clip,type Snapshot} from './api';
import {clipGroups} from './projectState';
import {Thumbnail,Select,type SelectOption} from './components';

type LibraryItem={key:string;clip:Clip;versions:Clip[];source?:string};
export function libraryItems(clips:Clip[]):LibraryItem[]{
 return [
  ...clipGroups(clips).flatMap(({path,raw,edited})=>{
   const versions=raw?[raw,...edited]:edited;
   const source=versions.find(clip=>clip.source?.trim())?.source?.trim();
   const unnamed=raw??edited.find(clip=>!clip.title);
   const named=edited.filter(clip=>clip.title).map(clip=>({key:clip.path,clip,versions,source}));
   return unnamed?[{key:path,clip:unnamed,versions,source},...named]:named;
  }),
  ...clips.filter(clip=>clip.is_movie).map(clip=>({key:clip.path,clip,versions:[clip]})),
 ];
}

function Cards({items,onOpen,onCopy,onDelete,picking=false,chosen=[],onToggle}:{items:LibraryItem[];onOpen?:(c:Clip)=>void;onCopy?:(c:Clip)=>void;onDelete?:(c:Clip)=>void;picking?:boolean;chosen?:string[];onToggle?:(path:string)=>void}){
 return <div className="library-grid">{items.map(({key,clip,versions})=>{
  const title=clipTitle(clip);
  const card=<button
   key={key}
   className={`media-card ${chosen.includes(clip.path)?'chosen':''}`}
   aria-pressed={picking?chosen.includes(clip.path):undefined}
   aria-label={picking?`Выбрать ${title}`:`Открыть ${title}`}
   onClick={()=>picking?onToggle?.(clip.path):onOpen?.(clip)}
  >
   <div className="media-cover">
    <Thumbnail clip={clip} seconds={Math.min(2,clip.duration*.2)}/>
    <span className="duration">{time(clip.duration)}</span>
    {!clip.viewed&&!clip.is_movie&&<span className="unread-dot" aria-label="Не просмотрен"/>}
    {picking&&<span className="pick-check">{chosen.includes(clip.path)&&<Check size={14}/>}</span>}
   </div>
   <span className="media-caption">
    <span title={title}>{title}</span>
    {versions.length>1&&<span className="version-count" title={`Версий: ${versions.length}`}><Layers2 size={13} aria-hidden="true"/>{versions.length}</span>}
   </span>
  </button>;
  if(!onCopy||!onDelete)return card;
  return <Menu key={key}>
   <MenuContextTrigger render={card}/>
   <MenuContent className="dropdown clip-menu" aria-label="Действия с клипом">
    <MenuItem value="open" onSelect={()=>onOpen?.(clip)}><Play size={15}/>Открыть</MenuItem>
    <MenuItem value="copy" onSelect={()=>onCopy(clip)}><Copy size={15}/>Скопировать</MenuItem>
    <MenuItem value="delete" className="danger-action" onSelect={()=>onDelete(clip)}><Trash2 size={15}/>Удалить</MenuItem>
   </MenuContent>
  </Menu>;
 })}</div>;
}
export function ClipLibraryGrid({items,status,onRetry,sourceFilter,sourceOptions,onSourceFilter,onOpen,onCopy,onDelete,onImport,onCapture,onError,state,scrollPosition}:{items:LibraryItem[];status:'loading'|'ready'|'failed';onRetry:()=>void;sourceFilter:string;sourceOptions:SelectOption[];onSourceFilter:(value:string)=>void;onOpen:(c:Clip)=>void;onCopy:(c:Clip)=>void;onDelete:(c:Clip)=>void;onImport:()=>void;onCapture:()=>void;onError:(e:string)=>void;state?:Snapshot;scrollPosition:RefObject<number>}){
 const host=useRef<HTMLDivElement>(null);useLayoutEffect(()=>{if(host.current)host.current.scrollTop=scrollPosition.current;},[scrollPosition,sourceFilter]);
 const count=items.length;const replayStatus=state?.replay_status??(state?.transitioning?'warming':state?.recording?'ready':'stopped');const captureLabel=replayStatus==='saving'?'Сохранение…':replayStatus==='warming'?'Подготовка…':replayStatus==='ready'?'Сохранить момент':'Включить запись';
 return <main className="library-page">
  <header className="page-header">
   <div className="library-title">
    <h1>Клипы</h1>{count>0&&<span className="item-count" title={`Клипов: ${count}`}>{count}</span>}
    {sourceOptions.length>1&&<Select value={sourceFilter} options={sourceOptions} label="Фильтр по источнику" onChange={onSourceFilter}/>}
   </div>
   <div className="library-actions">
    <button className="secondary-button" onClick={()=>void api.folder().catch(e=>onError(String(e)))}><FolderOpen size={16}/>Открыть папку</button>
    {count>0&&<button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button>}
   </div>
  </header>
  <div className="library-scroll" ref={host} onScroll={e=>{scrollPosition.current=e.currentTarget.scrollTop;}}>
   {count?<Cards items={items} onOpen={onOpen} onCopy={onCopy} onDelete={onDelete}/>
   :status==='loading'?<div className="empty-state library-status" role="status"><LoaderCircle size={20} className="spin" aria-hidden="true"/>Загрузка клипов…</div>
   :status==='failed'?<div className="empty-state library-status"><span>Не удалось загрузить клипы</span><button className="secondary-button" onClick={onRetry}>Повторить</button></div>
   :<div className="empty-library">
    <div className="empty-film" aria-hidden="true"><span><Film size={28} strokeWidth={1.5}/></span></div>
    <h2>Пока нет клипов</h2>
    <div className="empty-actions">
     <button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button>
     <button className="secondary-button" onClick={onCapture} disabled={!state||replayStatus==='warming'||replayStatus==='saving'}>
      {replayStatus==='ready'?<Zap size={16}/>:<Video size={16}/>}
      {captureLabel}
     </button>
    </div>
   </div>}
  </div>
 </main>;
}
export function ClipPicker({open,onOpenChange,clips,onAdd,onImport,limit=128}:{open:boolean;onOpenChange:(v:boolean)=>void;clips:Clip[];onAdd:(clips:Clip[])=>void;onImport:()=>void;limit?:number}){
 const [chosen,setChosen]=useState<string[]>([]);const toggle=(path:string)=>setChosen(v=>v.includes(path)?v.filter(p=>p!==path):v.length<limit?[...v,path]:v);const close=()=>{setChosen([]);onOpenChange(false);};
 return <Dialog open={open} onOpenChange={v=>{setChosen([]);onOpenChange(v);}}><DialogBackdrop className="dialog-backdrop"/><DialogContent className="clip-picker"><header className="dialog-header"><DialogTitle>Добавить клипы</DialogTitle><span className="spacer"/><button className="secondary-button" onClick={onImport}><Plus size={16}/>Импорт</button><DialogClose className="icon-button" aria-label="Закрыть выбор клипов"><X size={16}/></DialogClose></header><div className="picker-scroll">{clips.length?<Cards items={libraryItems(clips)} picking chosen={chosen} onToggle={toggle}/>:<div className="picker-empty"><button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button></div>}</div><footer className="picker-footer"><button className="secondary-button" onClick={close}>Отмена</button><button className="primary-button" disabled={!chosen.length} onClick={()=>{onAdd(chosen.map(path=>clips.find(c=>c.path===path)!).filter(Boolean));setChosen([]);onOpenChange(false);}}>Добавить{chosen.length?` ${chosen.length}`:''}</button></footer></DialogContent></Dialog>;
}
