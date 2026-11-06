import {useEffect,useRef,useState} from 'react';
import {time,trimAt,type Clip} from './api';
import type {Selection} from './Timeline';

function parseTime(value:string){const match=/^(\d+):([0-5]\d)(?:\.(\d{1,3}))?$/.exec(value.trim());return match?Number(match[1])*60+Number(match[2])+Number((match[3]??'').padEnd(3,'0'))/1000:undefined;}
function TimeField({label,value,onCommit,disabled}:{label:string;value:number;onCommit:(v:number)=>void;disabled:boolean}){
 const [draft,setDraft]=useState(time(value,true));const [invalid,setInvalid]=useState(false);const cancelled=useRef(false);
 useEffect(()=>{setDraft(time(value,true));setInvalid(false);},[value]);
 const commit=()=>{const parsed=parseTime(draft);setDraft(time(value,true));setInvalid(false);if(!cancelled.current&&parsed!==undefined)onCommit(parsed);cancelled.current=false;};
 return <label className="time-field"><span>{label}</span><input aria-label={label} aria-invalid={invalid} inputMode="decimal" value={draft} disabled={disabled} onChange={e=>{setDraft(e.target.value);setInvalid(parseTime(e.target.value)===undefined);}} onBlur={commit} onKeyDown={e=>{if(e.key==='Enter'){e.preventDefault();e.currentTarget.blur();}if(e.key==='Escape'){e.preventDefault();cancelled.current=true;e.currentTarget.blur();}}}/></label>;
}
export function TrimFields({clip,selection,onChange,disabled=false}:{clip:Clip;selection:Selection;onChange:(s:Selection)=>void;disabled?:boolean}){
 return <div className="trim-fields"><TimeField label="Начало" value={selection.start} disabled={disabled} onCommit={v=>onChange(trimAt('start',v,selection.start,selection.end,clip.duration,clip.fps))}/><TimeField label="Конец" value={selection.end} disabled={disabled} onCommit={v=>onChange(trimAt('end',v,selection.start,selection.end,clip.duration,clip.fps))}/><div className="duration-field"><span>Длительность</span><output>{time(selection.end-selection.start,true)}</output></div></div>;
}
