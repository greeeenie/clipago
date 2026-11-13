import {useEffect,useRef,useState,memo} from 'react';
import type {ButtonHTMLAttributes,ReactNode} from 'react';
import {Select as SdkSelect,SelectTrigger,SelectContent,SelectItem,SelectItemText,SelectItemIndicator,SelectGroup,SelectGroupLabel} from '@ui-sdk/react/select';
import {Slider as SdkSlider,SliderControl,SliderTrack,SliderIndicator,SliderThumb} from '@ui-sdk/react/slider';
import {Switch as SdkSwitch,SwitchThumb} from '@ui-sdk/react/switch';
import {Tooltip,TooltipTrigger,TooltipContent} from '@ui-sdk/react/tooltip';
import {Check,ChevronDown,Film,Rewind,LoaderCircle,Save} from 'lucide-react';
import {api,type Clip} from './api';

export function Logo({small=false}:{small?:boolean}){return <span className={`logo ${small?'small':''}`}><Rewind size={small?14:18} strokeWidth={2.4} fill="currentColor"/></span>;}
export function SaveAction({busy,onSave,label='Сохранить копию',disabled=false}:{busy:boolean;onSave:()=>void;label?:string;disabled?:boolean}){return busy?<div className="save-actions"><span className="export-progress" role="status"><LoaderCircle size={15} className="spin"/>Сохранение…</span><button className="secondary-button" onClick={()=>void api.cancel()}>Отменить</button></div>:<button className="primary-button" disabled={disabled} onClick={onSave}><Save size={15}/>{label}</button>;}
export function IconButton({label,hint=label,children,...props}:ButtonHTMLAttributes<HTMLButtonElement>&{label:string;hint?:string;children:ReactNode}){
  return <Tooltip delay={450}><TooltipTrigger className="icon-button" aria-label={label} {...props}>{children}</TooltipTrigger><TooltipContent className="tooltip" sideOffset={6}>{hint}</TooltipContent></Tooltip>;
}
export type SelectOption<T extends string=string>={value:T;label:string;icon?:ReactNode;group?:string};
export function Select<T extends string>({value,onChange,options,label,disabled=false}:{value:T;onChange:(v:T)=>void;options:SelectOption<T>[];label:string;disabled?:boolean}){
  const groups=new Map<string,SelectOption<T>[]>();
  const ungrouped=options.filter(option=>{
    if(!option.group)return true;
    const group=groups.get(option.group)??[];
    group.push(option);groups.set(option.group,group);
    return false;
  });
  const optionLabel=(option:SelectOption<T>)=>option.icon?<span className="select-icon-label">{option.icon}<span>{option.label}</span></span>:option.label;
  const item=(option:SelectOption<T>)=><SelectItem className="select-item" key={option.value} value={option.value} label={option.label}>
    <SelectItemText>{optionLabel(option)}</SelectItemText><SelectItemIndicator><Check size={14}/></SelectItemIndicator>
  </SelectItem>;
  const selected=options.find(option=>option.value===value);
  return <SdkSelect value={value} onValueChange={v=>{if(v!==null)onChange(v);}} disabled={disabled}>
   <SelectTrigger className="select-trigger" aria-label={label}><span>{selected?optionLabel(selected):''}</span><span><ChevronDown size={14}/></span></SelectTrigger>
   <SelectContent className="select-content" align="start" sideOffset={5}>
    {ungrouped.map(item)}
    {[...groups].map(([name,options])=><SelectGroup key={name} className="select-group"><SelectGroupLabel className="select-group-label">{name}</SelectGroupLabel>{options.map(item)}</SelectGroup>)}
   </SelectContent>
  </SdkSelect>;
}
export function Switch({value,onChange,label,disabled=false}:{value:boolean;onChange:(n:boolean)=>void;label:string;disabled?:boolean}){return <SdkSwitch className="switch" checked={value} onCheckedChange={onChange} disabled={disabled} aria-label={label}><SwitchThumb className="switch-thumb"/></SdkSwitch>;}
export function Slider({value,onChange,onCommit,min,max,step=1,label,valueText,disabled=false}:{value:number;onChange:(n:number)=>void;onCommit?:(n:number)=>void;min:number;max:number;step?:number;label:string;valueText?:string;disabled?:boolean}){
 return <SdkSlider className="slider" value={value} min={min} max={max} step={step} disabled={disabled} onValueChange={onChange} onValueCommit={onCommit}>
  <SliderControl className="slider-control"><SliderTrack className="slider-track"><SliderIndicator className="slider-range"/><SliderThumb className="slider-thumb" aria-label={label} aria-valuetext={valueText}/></SliderTrack></SliderControl>
 </SdkSlider>;
}
const cache=new Map<string,string>();const pending=new Map<string,Promise<string>>();
function getThumbnail(path:string,seconds:number){const key=`${path}@${seconds.toFixed(2)}`;if(cache.has(key))return Promise.resolve(cache.get(key)!);if(pending.has(key))return pending.get(key)!;
  const job=api.thumbnail(path,seconds).then(data=>{cache.set(key,data);while(cache.size>128)cache.delete(cache.keys().next().value!);return data;}).finally(()=>pending.delete(key));pending.set(key,job);return job;
}
export const Thumbnail=memo(function Thumbnail({clip,seconds=0}:{clip:Clip;seconds?:number}){
  const ref=useRef<HTMLSpanElement>(null);const [src,setSrc]=useState<string>();
  useEffect(()=>{let live=true;const observer=new IntersectionObserver(entries=>{if(entries.some(e=>e.isIntersecting)){observer.disconnect();getThumbnail(clip.path,seconds).then(src=>{if(live)setSrc(src);}).catch(()=>{});}},{rootMargin:'100px'});if(ref.current)observer.observe(ref.current);return()=>{live=false;observer.disconnect();};},[clip.path,seconds]);
  return <span className="thumbnail" ref={ref}>{src?<img src={src} alt="" draggable={false}/>:<Film size={18}/>}</span>;
});
