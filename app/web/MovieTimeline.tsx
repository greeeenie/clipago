import {useEffect,useRef,useState,type PointerEvent} from 'react';
import {Copy,GripVertical,Magnet,Minus,Plus,Redo2,Trash2,Undo2} from 'lucide-react';
import {time,filename,type Clip,type EditorHotkeys,type MovieSegment} from './api';
import {IconButton,Thumbnail,Slider} from './components';
import type {ClipDrop} from './clipDrag';
import {moveSegmentPlacement,segmentsAtPositions,type MovieTiming} from './movieTiming';
import {hotkeyLabel} from './editorHotkeys';

type Props={timing:MovieTiming;clips:Clip[];active:string;cursor:number;busy:boolean;canUndo:boolean;canRedo:boolean;canSplit:boolean;onUndo:()=>void;onRedo:()=>void;onSelect:(id:string)=>void;onSeek:(time:number)=>void;onDrop:(payload:ClipDrop,index:number)=>void;onMove:(segments:MovieSegment[],active:string)=>void;onAdd:()=>void;onDelete:()=>void;onDuplicate:()=>void;onSplit:()=>void;snapping:boolean;onSnapping:()=>void;hotkeys:EditorHotkeys;zoom:number;onZoom:(value:number)=>void};
function Razor(){return <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><g transform="rotate(-35 12 12)"><path d="M5 5h5v2h4V5h5v14h-5v-2h-4v2H5z"/><path d="M8 12h8"/></g></svg>;}
export function MovieTimeline({timing,clips,active,cursor,busy,canUndo,canRedo,canSplit,onUndo,onRedo,onSelect,onSeek,onDrop,onMove,onAdd,onDelete,onDuplicate,onSplit,snapping,onSnapping,hotkeys,zoom,onZoom}:Props){
 const host=useRef<HTMLDivElement>(null);const track=useRef<HTMLDivElement>(null);const [width,setWidth]=useState(800);
 const drag=useRef<{id:string;x:number;start:number;scroll:number;moved:boolean}|undefined>(undefined);const blockClick=useRef(false);const [draft,setDraft]=useState<{id:string;time:number}>();
 useEffect(()=>{if(typeof ResizeObserver==='undefined')return;const observer=new ResizeObserver(e=>setWidth(e[0].contentRect.width));if(host.current)observer.observe(host.current);return()=>observer.disconnect();},[]);
 const scale=Math.max(1,(width-64)/Math.max(1,timing.duration))*zoom;
 const placement=draft?moveSegmentPlacement(timing,draft.id,draft.time,snapping?10/scale:0,cursor):{parts:timing.parts,snapTime:undefined};const parts=placement.parts;
 const duration=Math.max(timing.duration,parts.at(-1)?.end??0);const canvas=Math.max(width,duration*scale+64);const selected=timing.parts.findIndex(p=>p.segment.id===active);const step=Math.max(1,Math.ceil(duration/Math.max(2,Math.floor(canvas/100))/5)*5);
 const hint=(label:string,id:keyof EditorHotkeys)=>`${label} · ${hotkeyLabel(hotkeys[id])}`;
 const seek=(e:PointerEvent)=>{if(busy||!track.current)return;onSeek(Math.max(0,Math.min(timing.duration,(e.clientX-track.current.getBoundingClientRect().left)/scale)));};
 const proposed=(e:PointerEvent)=>{const moving=drag.current!;return Math.max(0,moving.start+(e.clientX-moving.x+(host.current?.scrollLeft??0)-moving.scroll)/scale);};
 const begin=(e:PointerEvent<HTMLButtonElement>,id:string,start:number)=>{if(busy||e.button!==0)return;e.stopPropagation();drag.current={id,x:e.clientX,start,scroll:host.current?.scrollLeft??0,moved:false};e.currentTarget.setPointerCapture?.(e.pointerId);};
 const move=(e:PointerEvent<HTMLButtonElement>)=>{
  const moving=drag.current;if(!moving)return;if(!moving.moved&&Math.abs(e.clientX-moving.x)<6)return;e.preventDefault();moving.moved=true;
  const scroll=host.current;if(scroll){const bounds=scroll.getBoundingClientRect();if(e.clientX>bounds.right-32)scroll.scrollLeft+=16;else if(e.clientX<bounds.left+32)scroll.scrollLeft-=16;}
  setDraft({id:moving.id,time:proposed(e)});
 };
 const end=(e:PointerEvent<HTMLButtonElement>)=>{
  const moving=drag.current;if(!moving)return;
  if(moving.moved){e.preventDefault();blockClick.current=true;globalThis.setTimeout(()=>{blockClick.current=false;},0);const {parts}=moveSegmentPlacement(timing,moving.id,proposed(e),snapping?10/scale:0,cursor);onMove(segmentsAtPositions(parts),moving.id);}
  drag.current=undefined;setDraft(undefined);
 };
 const nativeDrop=(e:React.DragEvent,index:number)=>{e.preventDefault();e.stopPropagation();if(busy)return;const id=e.dataTransfer.getData('application/x-rewind-segment');const path=e.dataTransfer.getData('application/x-rewind-clip');if(id||path)onDrop({kind:id?'segment':'clip',value:id||path,label:''},index);};
 return <section className="movie-timeline" aria-label="Монтажная последовательность" data-movie-drop={timing.parts.length} onDragOver={e=>e.preventDefault()} onDrop={e=>nativeDrop(e,timing.parts.length)}>
 <div className="timeline-toolbar">
  <IconButton label="Отменить изменение" hint={hint('Отменить изменение','undo')} disabled={busy||!canUndo} onClick={onUndo}><Undo2 size={16}/></IconButton>
  <IconButton label="Повторить изменение" hint={hint('Повторить изменение','redo')} disabled={busy||!canRedo} onClick={onRedo}><Redo2 size={16}/></IconButton><span className="toolbar-divider"/>
  <IconButton label="Разделить по курсору" hint={hint('Разделить по курсору','split')} disabled={busy||!canSplit||timing.parts.length>=128} onClick={onSplit}><Razor/></IconButton>
  <IconButton label={`Удалить фрагмент ${selected+1}`} hint={hint('Удалить фрагмент','delete')} disabled={busy||selected<0} onClick={onDelete}><Trash2 size={16}/></IconButton>
  <IconButton label={`Дублировать фрагмент ${selected+1}`} hint={hint('Дублировать фрагмент','duplicate')} disabled={busy||selected<0||timing.parts.length>=128} onClick={onDuplicate}><Copy size={16}/></IconButton><span className="toolbar-divider"/>
  <IconButton label="Магнитизм" hint={hint('Магнитизм','snapping')} className="icon-button magnet-button" aria-pressed={snapping} disabled={busy} onClick={onSnapping}><Magnet size={17}/></IconButton><span className="spacer"/>
  <div className="timeline-zoom"><IconButton label="Уменьшить таймлайн" hint={hint('Уменьшить таймлайн','zoom_out')} onClick={()=>onZoom(Math.max(1,zoom-.5))}><Minus size={14}/></IconButton><Slider label="Масштаб таймлайна" value={zoom} min={1} max={8} step={.5} onChange={onZoom}/><IconButton label="Увеличить таймлайн" hint={hint('Увеличить таймлайн','zoom_in')} onClick={()=>onZoom(Math.min(8,zoom+.5))}><Plus size={14}/></IconButton></div><output className="movie-duration">{time(timing.duration)}</output>
 </div>
 <div className="movie-timeline-scroll" ref={host}><div className="movie-timeline-canvas" ref={track} style={{width:canvas}}>
  <div className="movie-ruler" onPointerDown={e=>{e.currentTarget.setPointerCapture(e.pointerId);seek(e);}} onPointerMove={e=>{if(e.buttons===1)seek(e);}}>{Array.from({length:Math.floor(duration/step)+1},(_,i)=><span key={i} style={{left:i*step*scale}}>{time(i*step)}</span>)}</div>
  <div className="movie-segments" onPointerDown={e=>{if(e.target===e.currentTarget)seek(e);}}>
   {parts.map(({segment,start,duration},index)=>{const clip=clips.find(c=>c.path===segment.path);return <article key={segment.id} className={`movie-segment ${active===segment.id?'active':''} ${draft?.id===segment.id?'moving':''}`} style={{left:start*scale,width:Math.max(1,duration*scale)}} data-movie-drop={index} onDragOver={e=>e.preventDefault()} onDrop={e=>nativeDrop(e,index)}>
    <button className="segment-cover" draggable={false} aria-label={`Выбрать фрагмент ${index+1}: ${filename(segment.path)}`} onPointerDown={e=>begin(e,segment.id,start)} onPointerMove={move} onPointerUp={end} onPointerCancel={()=>{drag.current=undefined;setDraft(undefined);}} onClick={()=>{if(blockClick.current)return;onSelect(segment.id);}}>{clip?<Thumbnail clip={clip} seconds={(segment.start+segment.end)/2}/>:<span className="missing-media">Недоступен</span>}<GripVertical className="segment-grip start" size={13}/><GripVertical className="segment-grip end" size={13}/></button>
    <div className="segment-caption"><span>{clip?.source??filename(segment.path)}</span><span>{time(duration)}</span></div>
   </article>;})}
   <button className="add-segment" style={{left:(parts.at(-1)?.end??0)*scale}} data-movie-drop={timing.parts.length} aria-label="Добавить клипы" onClick={onAdd} disabled={busy||timing.parts.length>=128}><Plus size={22}/></button>
  </div>
  {placement.snapTime!==undefined&&<div className="timeline-snap-guide" aria-hidden="true" style={{left:placement.snapTime*scale}}/>}
  {timing.parts.length>0&&<button className="movie-playhead" role="slider" aria-label="Позиция в мувике" aria-valuemin={0} aria-valuemax={timing.duration} aria-valuenow={cursor} style={{left:cursor*scale}} disabled={busy} onPointerDown={e=>{e.currentTarget.setPointerCapture(e.pointerId);seek(e);}} onPointerMove={e=>{if(e.buttons===1)seek(e);}} onKeyDown={e=>{if(['ArrowLeft','ArrowRight','Home','End'].includes(e.key)){e.preventDefault();onSeek(e.key==='Home'?0:e.key==='End'?timing.duration:cursor+(e.key==='ArrowLeft'?-1:1)*(e.shiftKey?1:1/timing.fps));}}}><span style={cursor*scale<40?{left:6,transform:'none'}:undefined}>{time(cursor,true)}</span></button>}
 </div></div>
 </section>;
}
