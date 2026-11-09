import {useLayoutEffect,useRef,useState,type RefObject} from 'react';
import {Dialog} from 'radix-ui';
import {Check,Film,FolderOpen,Layers2,Plus,Video,X,Zap} from 'lucide-react';
import {api,clipTitle,time,type Clip,type Snapshot} from './api';
import {clipGroups} from './projectState';
import {Thumbnail,IconButton} from './components';

export function libraryItems(clips:Clip[]){return [...clipGroups(clips).map(g=>({key:g.path,clip:g.raw??g.edited[0],versions:[...(g.raw?[g.raw]:[]),...g.edited]})),...clips.filter(c=>c.is_movie).map(clip=>({key:clip.path,clip,versions:[clip]}))];}
function Cards({clips,onOpen,picking=false,chosen=[],onToggle}:{clips:Clip[];onOpen?:(c:Clip)=>void;picking?:boolean;chosen?:string[];onToggle?:(path:string)=>void}){
 return <div className="library-grid">{libraryItems(clips).map(({key,clip,versions})=>{
  const title=clipTitle(clip);
  return <button
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
 })}</div>;
}
export function ClipLibraryGrid({clips,onOpen,onImport,onCapture,onError,state,scrollPosition}:{clips:Clip[];onOpen:(c:Clip)=>void;onImport:()=>void;onCapture:()=>void;onError:(e:string)=>void;state?:Snapshot;scrollPosition:RefObject<number>}){
 const host=useRef<HTMLDivElement>(null);useLayoutEffect(()=>{if(host.current)host.current.scrollTop=scrollPosition.current;},[scrollPosition]);
 const count=libraryItems(clips).length;const status=state?.replay_status??(state?.transitioning?'warming':state?.recording?'ready':'stopped');const captureLabel=status==='saving'?'Сохранение…':status==='warming'?'Подготовка…':status==='ready'?'Сохранить момент':'Включить запись';
 return <main className="library-page">
  <header className="page-header">
   <div className="library-title"><h1>Клипы</h1>{count>0&&<span className="item-count" title={`Клипов: ${count}`}>{count}</span>}</div>
   <div className="library-actions">
    <button className="secondary-button" onClick={()=>void api.folder().catch(e=>onError(String(e)))}><FolderOpen size={16}/>Открыть папку</button>
    {count>0&&<button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button>}
   </div>
  </header>
  <div className="library-scroll" ref={host} onScroll={e=>{scrollPosition.current=e.currentTarget.scrollTop;}}>
   {count?<Cards clips={clips} onOpen={onOpen}/>:<div className="empty-library">
    <div className="empty-film" aria-hidden="true"><span><Film size={28} strokeWidth={1.5}/></span></div>
    <h2>Пока нет клипов</h2>
    <div className="empty-actions">
     <button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button>
     <button className="secondary-button" onClick={onCapture} disabled={!state||status==='warming'||status==='saving'}>
      {status==='ready'?<Zap size={16}/>:<Video size={16}/>}
      {captureLabel}
     </button>
    </div>
   </div>}
  </div>
 </main>;
}
export function ClipPicker({open,onOpenChange,clips,onAdd,onImport,limit=128}:{open:boolean;onOpenChange:(v:boolean)=>void;clips:Clip[];onAdd:(clips:Clip[])=>void;onImport:()=>void;limit?:number}){
 const [chosen,setChosen]=useState<string[]>([]);const toggle=(path:string)=>setChosen(v=>v.includes(path)?v.filter(p=>p!==path):v.length<limit?[...v,path]:v);const close=()=>{setChosen([]);onOpenChange(false);};
 return <Dialog.Root open={open} onOpenChange={v=>{setChosen([]);onOpenChange(v);}}><Dialog.Portal><Dialog.Overlay className="dialog-backdrop"/><Dialog.Content className="clip-picker" aria-describedby={undefined}><header className="dialog-header"><Dialog.Title>Добавить клипы</Dialog.Title><span className="spacer"/><button className="secondary-button" onClick={onImport}><Plus size={16}/>Импорт</button><Dialog.Close asChild><IconButton label="Закрыть выбор клипов"><X size={16}/></IconButton></Dialog.Close></header><div className="picker-scroll">{clips.length?<Cards clips={clips} picking chosen={chosen} onToggle={toggle}/>:<div className="picker-empty"><button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button></div>}</div><footer className="picker-footer"><button className="secondary-button" onClick={close}>Отмена</button><button className="primary-button" disabled={!chosen.length} onClick={()=>{onAdd(chosen.map(path=>clips.find(c=>c.path===path)!).filter(Boolean));setChosen([]);onOpenChange(false);}}>Добавить{chosen.length?` ${chosen.length}`:''}</button></footer></Dialog.Content></Dialog.Portal></Dialog.Root>;
}
