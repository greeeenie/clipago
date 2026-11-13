import {useEffect,useMemo,useRef,useState} from 'react';
import {Dialog,DialogBackdrop,DialogContent,DialogTitle,DialogClose} from '@ui-sdk/react/dialog';
import {listen} from '@tauri-apps/api/event';
import {Film,Pencil,Plus,Save,SlidersHorizontal,X} from 'lucide-react';
import {api,defaultConfig,filename,time,type Clip,type ExportConfig,type MovieProject,type Snapshot} from './api';
import {IconButton,SaveAction} from './components';
import {Editor} from './Editor';
import {ExportPanel} from './ExportPanel';
import {TrimFields} from './TrimFields';
import {ClipPicker} from './ClipLibraryGrid';
import {MovieTimeline} from './MovieTimeline';
import {MovieGapPreview} from './MovieGapPreview';
import {movieTiming,moveSegmentPlacement,resolveMovieTime,segmentsAtPositions,splitSegment} from './movieTiming';
import {useProjectHistory} from './useProjectHistory';
import {editorHotkeys,hotkeyLabel,keyCombination} from './editorHotkeys';
import type {ClipDrop} from './clipDrag';
import type {Selection} from './Timeline';

type Props={clips:Clip[];state:Snapshot;project:MovieProject;setProject:(p:MovieProject)=>void;onImport:()=>void;onViewed:(c:Clip)=>void;onSettings:()=>void;onError:(s:string)=>void;onSaved:(c:Clip)=>void;onBusy:(busy:boolean)=>void;suspended:boolean};
export function MovieWorkspace({clips,state,project,setProject,onImport,onViewed,onError,onSaved,onBusy,suspended}:Props){
 const [active,setActive]=useState(project.segments[0]?.id??'');const [cursor,setCursor]=useState(0);const [playing,setPlaying]=useState(false);
 const [inspector,setInspector]=useState(true);const [exportOpen,setExportOpen]=useState(false);const [picker,setPicker]=useState(false);const [busy,setBusy]=useState(false);const [zoom,setZoom]=useState(1);const [name,setName]=useState(project.name);
 const saving=useRef(false);const nameInput=useRef<HTMLInputElement>(null);const history=useProjectHistory(project,setProject);
 const hardware=state.encoders.some(e=>!['Auto','Cpu'].includes(e));const hotkeys=editorHotkeys(state.settings);const snapping=project.snapping!==false;
 const timing=useMemo(()=>movieTiming(project,clips),[project,clips]);
 const selected=timing.parts.find(p=>p.segment.id===active);const selectedClip=clips.find(c=>c.path===selected?.segment.path);
 const resolved=resolveMovieTime(timing,cursor);const preview=resolved?.part;const previewClip=clips.find(c=>c.path===preview?.segment.path);
 const height=Math.min(2160,Math.max(90,...project.segments.map(s=>clips.find(c=>c.path===s.path)?.height??1080)));
 const hasAudio=project.segments.some(s=>s.config.audioTracks.length>0&&clips.find(c=>c.path===s.path)?.has_audio);
 const summary:Clip={path:'movie',duration:timing.duration,bytes:project.segments.reduce((n,s)=>n+(clips.find(c=>c.path===s.path)?.bytes??0)*(s.end-s.start)/(clips.find(c=>c.path===s.path)?.duration??1),0),width:Math.round(height*16/9/2)*2,height,fps:timing.fps,has_audio:hasAudio,audio_tracks:hasAudio?[{index:0,name:'Звук'}]:[],audio_mix_index:null,source:null,aspect_ratio:16/9,original_path:null,viewed:true,is_movie:true};
 const rawOutput=project.output??defaultConfig(summary,hardware);const output={...rawOutput,audioTracks:hasAudio?[0]:[]};
 const gapEnd=timing.parts.find(p=>p.start>cursor+1e-7)?.start??timing.duration;

 useEffect(()=>{setName(project.name);},[project.name]);
 useEffect(()=>{if(!timing.parts.some(p=>p.segment.id===active))setActive(timing.parts[0]?.segment.id??'');setCursor(value=>Math.min(value,timing.duration));},[project.segments,timing.duration,active]);
 useEffect(()=>{if(suspended||picker||exportOpen)setPlaying(false);},[suspended,picker,exportOpen]);
 useEffect(()=>{
  const hidden=()=>{if(document.hidden)setPlaying(false);};document.addEventListener('visibilitychange',hidden);
  let live=true;let off:(()=>void)|undefined;void listen('app-hidden',()=>setPlaying(false)).then(fn=>{if(live)off=fn;else fn();});
  return()=>{live=false;off?.();document.removeEventListener('visibilitychange',hidden);};
 },[]);
 useEffect(()=>{
  if(!playing||preview||busy||suspended||picker||exportOpen||cursor>=timing.duration)return;
  const started=performance.now();const from=cursor;let frame=0;let last=0;
  const tick=(now:number)=>{if(document.hidden){setPlaying(false);return;}const next=Math.min(gapEnd,from+(now-started)/1000);if(now-last>35||next>=gapEnd){last=now;setCursor(next);}if(next<gapEnd)frame=requestAnimationFrame(tick);};
  frame=requestAnimationFrame(tick);return()=>cancelAnimationFrame(frame);
 },[playing,preview?.segment.id,gapEnd,busy,suspended,picker,exportOpen,timing.duration]);

 const commit=(next:MovieProject,nextActive=active,nextCursor=cursor)=>{if(busy)return;history.commit(next);setActive(nextActive);setCursor(Math.max(0,Math.min(movieTiming(next,clips).duration,nextCursor)));setPlaying(false);};
 const focus=(id:string)=>{if(busy)return;const part=timing.parts.find(p=>p.segment.id===id);if(part){setActive(id);setCursor(part.start);setPlaying(false);setInspector(true);}};
 const seek=(value:number)=>{setPlaying(false);setCursor(Math.max(0,Math.min(timing.duration,value)));};
 const togglePlay=()=>{if(busy||!timing.parts.length)return;if(!playing&&cursor>=timing.duration)setCursor(0);setPlaying(value=>!value);};
 const openExport=()=>{if(!timing.parts.length||busy)return;setPlaying(false);setExportOpen(true);};
 const toggleInspector=()=>{if(selected)setInspector(value=>!value);};
 const toggleSnapping=()=>{if(!busy)setProject({...project,snapping:!snapping});};
 const add=(items:Clip[],index=project.segments.length)=>{
  if(busy)return;const available=128-project.segments.length;
  const additions=items.slice(0,available).map(c=>({id:crypto.randomUUID(),path:c.path,start:0,end:c.duration,gapBefore:0,config:defaultConfig(c,hardware)}));if(!additions.length)return;
  const segments=[...project.segments];segments.splice(index,0,...additions);const next={...project,segments};
  const start=movieTiming(next,clips).parts.find(p=>p.segment.id===additions[0].id)?.start??0;commit(next,additions[0].id,start);setInspector(true);items.slice(0,available).forEach(onViewed);
 };
 const drop=(payload:ClipDrop,index:number)=>{
  if(busy)return;if(payload.kind==='segment'){const {parts}=moveSegmentPlacement(timing,payload.value,timing.parts[index]?.start??timing.duration);commit({...project,segments:segmentsAtPositions(parts)},payload.value,parts.find(p=>p.segment.id===payload.value)?.start??cursor);}
  else{const clip=clips.find(c=>c.path===payload.value);if(clip)add([clip],index);}
 };
 const move=(segments:MovieProject['segments'],id:string)=>{const next={...project,segments};commit(next,id,movieTiming(next,clips).parts.find(p=>p.segment.id===id)?.start??cursor);};
 const duplicate=()=>{
  if(!selected||busy||project.segments.length>=128)return;const index=project.segments.findIndex(s=>s.id===active);const next=structuredClone(selected.segment);next.id=crypto.randomUUID();next.gapBefore=0;
  const segments=[...project.segments];segments.splice(index+1,0,next);commit({...project,segments},next.id,selected.end);setInspector(true);
 };
 const remove=()=>{
  if(!selected||busy)return;const parts=timing.parts.filter(p=>p!==selected);const nextActive=parts.find(p=>p.start>=selected.start)??parts.at(-1);
  commit({...project,segments:segmentsAtPositions(parts)},nextActive?.segment.id??'',cursor);
 };
 const split=()=>{
  if(!preview||!resolved||busy||project.segments.length>=128)return;const source=clips.find(c=>c.path===preview.segment.path);if(!source)return;
  const pieces=splitSegment(preview.segment,resolved.sourceTime,source.fps);if(!pieces)return;
  commit({...project,segments:project.segments.flatMap(s=>s.id===preview.segment.id?pieces:[s])},pieces[1].id,cursor);setInspector(true);
 };
 const changeConfig=(config:ExportConfig)=>{if(selected)commit({...project,segments:project.segments.map(s=>s.id===active?{...s,config}:s)});};
 const changeRange=(selection:Selection)=>{if(selected)commit({...project,segments:project.segments.map(s=>s.id===active?{...s,start:selection.start,end:selection.end}:s)},active,selected.start);};
 const undo=()=>{if(!busy){setPlaying(false);history.undo();}};const redo=()=>{if(!busy){setPlaying(false);history.redo();}};
 useEffect(()=>{
  const key=(e:KeyboardEvent)=>{
   if(e.defaultPrevented||e.repeat||suspended||busy||picker||exportOpen||(e.target as HTMLElement).closest('input,textarea,select,[contenteditable="true"],[role="dialog"],[role="combobox"]'))return;
   const combination=keyCombination(e);const action=(Object.keys(hotkeys) as (keyof typeof hotkeys)[]).find(id=>hotkeys[id]===combination);if(!action)return;e.preventDefault();
   ({split,duplicate,delete:remove,snapping:toggleSnapping,inspector:toggleInspector,export:openExport,undo,redo,play:togglePlay,zoom_in:()=>setZoom(value=>Math.min(8,value+.5)),zoom_out:()=>setZoom(value=>Math.max(1,value-.5))})[action]();
  };globalThis.addEventListener('keydown',key);return()=>globalThis.removeEventListener('keydown',key);
 });
 const render=async()=>{
  if(!project.segments.length||saving.current)return;saving.current=true;setBusy(true);onBusy(true);setPlaying(false);
  const snapshot=structuredClone(project);try{const missing=snapshot.segments.find(s=>!clips.some(c=>c.path===s.path));if(missing)throw new Error(`Клип недоступен: ${filename(missing.path)}`);
   const saved=await api.movie(snapshot.segments,clips,structuredClone(output),summary,snapshot.name);if(saved){onSaved(saved);setExportOpen(false);}
  }catch(e){onError(String(e));}finally{saving.current=false;setBusy(false);onBusy(false);}
 };
 const selectedRange:Selection={start:selected?.segment.start??0,end:selected?.segment.end??0,cursor:selected?.segment.start??0};
 const splitFrame=1/Math.max(1,previewClip?.fps??timing.fps);const splitTime=Math.round((resolved?.sourceTime??0)/splitFrame)*splitFrame;
 const canSplit=!!(preview&&splitTime-preview.segment.start>=splitFrame-1e-6&&preview.segment.end-splitTime>=splitFrame-1e-6);
 return <div className="main-layout movie-workspace"><main className="main-editor movie-editor"><header className="editor-header">
  <div className="project-title"><input ref={nameInput} style={{width:Math.min(30,Math.max(10,name.length))+'ch'}} className="project-name" aria-label="Название мувика" value={name} disabled={busy} maxLength={80} onChange={e=>setName(e.target.value)} onBlur={e=>{const value=e.currentTarget.value.trim()||'Новый мувик';setName(value);commit({...project,name:value});}} onKeyDown={e=>{if(e.key==='Enter')e.currentTarget.blur();if(e.key==='Escape'){e.currentTarget.value=project.name;setName(project.name);e.currentTarget.blur();}}}/><IconButton label="Переименовать мувик" disabled={busy} onClick={()=>{nameInput.current?.focus();nameInput.current?.select();}}><Pencil size={14}/></IconButton></div><span className="spacer"/>
  <IconButton label={inspector?'Скрыть настройки фрагмента':'Показать настройки фрагмента'} hint={`Настройки фрагмента · ${hotkeyLabel(hotkeys.inspector)}`} aria-pressed={inspector} onClick={toggleInspector} disabled={!selected||busy}><SlidersHorizontal size={17}/></IconButton>
  <button className="primary-button" onClick={openExport} disabled={!timing.parts.length||busy}><Save size={15}/>Экспорт видео</button>
 </header>
 {preview&&previewClip&&resolved?<Editor key={preview.segment.id} clip={previewClip} selection={{start:preview.segment.start,end:preview.segment.end,cursor:resolved.sourceTime}} audioTracks={preview.segment.config.audioTracks} onChange={selection=>setCursor(Math.min(preview.end,preview.start+selection.cursor-preview.segment.start))} showTimeline={false} displayTime={{cursor,duration:timing.duration}} autoPlay={playing} keyboardPlayback={false} onPlaybackChange={value=>{if(value&&cursor>=timing.duration)setCursor(0);setPlaying(value);}} onRangeEnd={()=>{setCursor(preview.end);if(preview.end>=timing.duration)setPlaying(false);}} busy={busy} suspended={suspended||picker||exportOpen} onViewed={()=>onViewed(previewClip)}/>:timing.parts.length?<MovieGapPreview cursor={cursor} duration={timing.duration} playing={playing} onToggle={togglePlay}/>:<div className="empty-state"><Film size={40} strokeWidth={1}/><button className="primary-button" onClick={()=>setPicker(true)}><Plus size={16}/>Добавить клипы</button></div>}
 <MovieTimeline timing={timing} clips={clips} active={active} cursor={cursor} busy={busy} canUndo={history.canUndo} canRedo={history.canRedo} canSplit={canSplit} onUndo={undo} onRedo={redo} onSelect={focus} onSeek={seek} onDrop={drop} onMove={move} onAdd={()=>{setPlaying(false);setPicker(true);}} onDelete={remove} onDuplicate={duplicate} onSplit={split} snapping={snapping} onSnapping={toggleSnapping} hotkeys={hotkeys} zoom={zoom} onZoom={setZoom}/>
 </main>
 {inspector&&selected&&selectedClip&&<aside className="inspector fragment-inspector" aria-label="Настройки фрагмента"><header className="panel-header"><h2 title={filename(selectedClip.path)}>{filename(selectedClip.path)}</h2></header><ExportPanel embedded mode="segment" clip={selectedClip} config={selected.segment.config} setConfig={changeConfig} start={selected.segment.start} end={selected.segment.end} state={state} busy={busy} onClose={()=>{}} onExport={()=>{}} advancedContent={<TrimFields clip={selectedClip} selection={selectedRange} onChange={changeRange} disabled={busy}/>}/></aside>}
 <ClipPicker open={picker} onOpenChange={setPicker} clips={clips} onAdd={items=>add(items)} onImport={onImport} limit={128-project.segments.length}/>
 <Dialog open={exportOpen} onOpenChange={(value,details)=>{if(busy)details.cancel();else setExportOpen(value);}}><DialogBackdrop className="dialog-backdrop"/><DialogContent className="movie-export-dialog"><header className="dialog-header"><DialogTitle>Экспорт видео</DialogTitle><DialogClose className="icon-button" aria-label="Закрыть экспорт" disabled={busy}><X size={17}/></DialogClose></header><div className="movie-export-name"><span>{project.name}</span><span>{time(timing.duration)}</span></div><ExportPanel embedded mode="movie" clip={summary} config={output} setConfig={value=>commit({...project,output:value})} start={0} end={timing.duration} state={state} busy={busy} onClose={()=>{}} onExport={()=>{}}/><footer className="export-dialog-footer"><SaveAction label="Экспортировать" busy={busy} onSave={()=>void render()}/></footer></DialogContent></Dialog>
 </div>;
}
