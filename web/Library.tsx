import {FolderOpen,Plus,Settings,Film} from 'lucide-react';
import {api,filename,time,type Clip} from './api';
import {clipDrag} from './clipDrag';
import {clipGroups} from './projectState';
import {IconButton,Thumbnail} from './components';
export function Library({clips,selected,onSelect,onImport,onSettings,onError,onAdd}:{clips:Clip[];selected?:string;onSelect:(clip:Clip)=>void;onImport:()=>void;onSettings:()=>void;onError:(e:string)=>void;onAdd?:(clip:Clip)=>void}){
 const groups=clipGroups(clips);
 return <aside className="library"><div className="library-heading"><h2>Клипы</h2><IconButton label="Добавить видео" onClick={onImport}><Plus size={17}/></IconButton></div><div className="clip-list">
 {groups.map(({path,raw,edited})=>{const latest=edited[0];const current=[raw,...edited].find(c=>c?.path===selected)??raw??latest;return <article className={`clip-card ${[raw,...edited].some(c=>c?.path===selected)?'selected':''}`} key={path}>
 <button className="clip-item" onClick={()=>onSelect(current)} {...clipDrag({kind:'clip',value:current.path,label:current.source??filename(current.path)})} onDragStart={e=>{e.dataTransfer.setData('application/x-rewind-clip',current.path);e.dataTransfer.effectAllowed='copy';}} title={filename(current.path)}><div className="clip-cover"><Thumbnail clip={current} seconds={Math.min(2,current.duration*.2)}/><span className="duration">{time(current.duration)}</span>{raw&&!raw.viewed&&<span className="unread-dot" aria-label="Не просмотрен" title="Не просмотрен"/>}</div><span className="clip-name">{current.source??filename(path)}</span>{current.source&&<span className="clip-source">{filename(path)}</span>}</button>
 <div className="clip-versions"><button disabled={!raw} className={current.path===raw?.path?'active':''} onClick={()=>raw&&onSelect(raw)}>Исходник</button>{latest&&<button className={!!current.original_path?'active':''} onClick={()=>onSelect(latest)}>Обработан</button>}{onAdd&&<IconButton label={`Добавить ${filename(current.path)} в монтаж`} onClick={()=>onAdd(current)}><Plus size={13}/></IconButton>}</div>
 </article>;})}
 {!groups.length&&<div className="library-empty"><Film size={20}/><span>Нет клипов</span></div>}</div><footer className="library-footer"><IconButton label="Открыть папку клипов" onClick={()=>void api.folder().catch(e=>onError(String(e)))}><FolderOpen size={17}/></IconButton><span className="spacer"/><IconButton label="Настройки" onClick={onSettings}><Settings size={17}/></IconButton></footer></aside>;
}
