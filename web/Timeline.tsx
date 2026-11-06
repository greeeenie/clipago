import {useRef,useState} from 'react';
import type {PointerEvent,KeyboardEvent} from 'react';
import {Thumbnail} from './components';
import {time,trimAt,type Clip} from './api';
type Kind='start'|'end'|'seek';
export type Selection={start:number;end:number;cursor:number};
export function Timeline({clip,selection,onChange,compact=false,disabled=false}:{clip:Clip;selection:Selection;onChange:(s:Selection)=>void;compact?:boolean;disabled?:boolean}){
  const track=useRef<HTMLDivElement>(null);const drag=useRef<Kind|null>(null);const [active,setActive]=useState<Kind|null>(null);const fps=clip.fps||30;
  const update=(kind:Kind,value:number)=>onChange(trimAt(kind,value,selection.start,selection.end,clip.duration,fps));
  const point=(e:PointerEvent)=>{const bounds=track.current!.getBoundingClientRect();return (e.clientX-bounds.left)/bounds.width*clip.duration;};
  const begin=(e:PointerEvent,kind:Kind)=>{if(disabled||e.button!==0)return;e.preventDefault();e.stopPropagation();drag.current=kind;setActive(kind);e.currentTarget.setPointerCapture(e.pointerId);update(kind,point(e));};
  const move=(e:PointerEvent)=>{if(drag.current)update(drag.current,point(e));};
  const end=()=>{drag.current=null;setActive(null);};
  const key=(e:KeyboardEvent,kind:Kind)=>{if(!['ArrowLeft','ArrowRight','Home','End'].includes(e.key))return;e.preventDefault();e.stopPropagation();const current=kind==='start'?selection.start:kind==='end'?selection.end:selection.cursor;update(kind,e.key==='Home'?0:e.key==='End'?clip.duration:current+(e.key==='ArrowLeft'?-1:1)*(e.shiftKey?1:1/fps));setActive(kind);};
  const pct=(n:number)=>`${Math.max(0,Math.min(100,n/clip.duration*100))}%`;
  return <div className={`timeline ${compact?'compact':''}`}>
    <div className="timeline-track" ref={track} onPointerDown={e=>begin(e,'seek')} onPointerMove={move} onPointerUp={end} onPointerCancel={end} aria-label="Таймлайн">
      <div className="filmstrip">{Array.from({length:8},(_,i)=><Thumbnail key={`${clip.path}:${i}`} clip={clip} seconds={Math.min(clip.duration-.05,clip.duration*(i+.5)/8)}/>)}</div>
      <div className="range-mask" style={{left:0,width:pct(selection.start)}}/><div className="range-mask" style={{left:pct(selection.end),right:0}}/>
      <div className="range-selected" style={{left:pct(selection.start),width:pct(selection.end-selection.start)}}/>
      {(['start','end'] as const).map(kind=><button key={kind} className={`trim-handle ${kind}`} role="slider" aria-label={kind==='start'?'Начало фрагмента':'Конец фрагмента'} aria-valuemin={0} aria-valuemax={clip.duration} aria-valuenow={selection[kind]} aria-valuetext={time(selection[kind],true)} disabled={disabled} style={{left:pct(selection[kind])}} onPointerDown={e=>begin(e,kind)} onPointerMove={move} onPointerUp={end} onPointerCancel={end} onKeyDown={e=>key(e,kind)} onBlur={()=>setActive(null)}><i/>{active===kind&&<span className="time-bubble">{time(selection[kind],true)}</span>}</button>)}
      <button className="playhead" role="slider" aria-label="Позиция воспроизведения" aria-valuemin={0} aria-valuemax={clip.duration} aria-valuenow={selection.cursor} disabled={disabled} style={{left:pct(selection.cursor)}} onPointerDown={e=>begin(e,'seek')} onPointerMove={move} onPointerUp={end} onPointerCancel={end} onKeyDown={e=>key(e,'seek')} onBlur={()=>setActive(null)}>{active==='seek'&&<span className="time-bubble">{time(selection.cursor,true)}</span>}</button>
    </div>
    {!compact&&<div className="timeline-scale">{[0,.25,.5,.75,1].map(n=><span key={n}>{time(clip.duration*n)}</span>)}</div>}
  </div>;
}
