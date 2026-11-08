import {useEffect,useRef,useState,memo} from 'react';
import type {ButtonHTMLAttributes,ReactNode} from 'react';
import {Select as RSelect,Slider as RSlider,Switch as RSwitch,Tooltip} from 'radix-ui';
import {Check,ChevronDown,Film,Rewind,LoaderCircle,Save} from 'lucide-react';
import {api,type Clip} from './api';

export function Logo({small=false}:{small?:boolean}){return <span className={`logo ${small?'small':''}`}><Rewind size={small?14:18} strokeWidth={2.4} fill="currentColor"/></span>;}
export function SaveAction({busy,onSave,label='Сохранить копию',disabled=false}:{busy:boolean;onSave:()=>void;label?:string;disabled?:boolean}){return busy?<div className="save-actions"><span className="export-progress" role="status"><LoaderCircle size={15} className="spin"/>Сохранение…</span><button className="secondary-button" onClick={()=>void api.cancel()}>Отменить</button></div>:<button className="primary-button" disabled={disabled} onClick={onSave}><Save size={15}/>{label}</button>;}
export function IconButton({label,hint=label,children,...props}:ButtonHTMLAttributes<HTMLButtonElement>&{label:string;hint?:string;children:ReactNode}){
  return <Tooltip.Root delayDuration={450}><Tooltip.Trigger asChild><button className="icon-button" aria-label={label} {...props}>{children}</button></Tooltip.Trigger><Tooltip.Portal><Tooltip.Content className="tooltip" sideOffset={6}>{hint}</Tooltip.Content></Tooltip.Portal></Tooltip.Root>;
}
export function Select<T extends string>({value,onChange,options,label,disabled=false}:{value:T;onChange:(v:T)=>void;options:{value:T;label:string}[];label:string;disabled?:boolean}){
  return <RSelect.Root value={value} onValueChange={v=>onChange(v as T)} disabled={disabled}><RSelect.Trigger className="select-trigger" aria-label={label}><RSelect.Value/><RSelect.Icon><ChevronDown size={14}/></RSelect.Icon></RSelect.Trigger><RSelect.Portal><RSelect.Content className="select-content" position="popper" sideOffset={5}><RSelect.Viewport>{options.map(option=><RSelect.Item className="select-item" key={option.value} value={option.value}><RSelect.ItemText>{option.label}</RSelect.ItemText><RSelect.ItemIndicator><Check size={14}/></RSelect.ItemIndicator></RSelect.Item>)}</RSelect.Viewport></RSelect.Content></RSelect.Portal></RSelect.Root>;
}
export function Switch({value,onChange,label,disabled=false}:{value:boolean;onChange:(n:boolean)=>void;label:string;disabled?:boolean}){return <RSwitch.Root className="switch" checked={value} onCheckedChange={onChange} disabled={disabled} aria-label={label}><RSwitch.Thumb className="switch-thumb"/></RSwitch.Root>;}
export function Slider({value,onChange,onCommit,min,max,step=1,label,valueText,disabled=false}:{value:number;onChange:(n:number)=>void;onCommit?:(n:number)=>void;min:number;max:number;step?:number;label:string;valueText?:string;disabled?:boolean}){return <RSlider.Root className="slider" value={[value]} min={min} max={max} step={step} disabled={disabled} onValueChange={v=>onChange(v[0])} onValueCommit={v=>onCommit?.(v[0])}><RSlider.Track className="slider-track"><RSlider.Range className="slider-range"/></RSlider.Track><RSlider.Thumb className="slider-thumb" aria-label={label} aria-valuetext={valueText}/></RSlider.Root>;}
const cache=new Map<string,string>();const pending=new Map<string,Promise<string>>();
function getThumbnail(path:string,seconds:number){const key=`${path}@${seconds.toFixed(2)}`;if(cache.has(key))return Promise.resolve(cache.get(key)!);if(pending.has(key))return pending.get(key)!;
  const job=api.thumbnail(path,seconds).then(data=>{cache.set(key,data);while(cache.size>128)cache.delete(cache.keys().next().value!);return data;}).finally(()=>pending.delete(key));pending.set(key,job);return job;
}
export const Thumbnail=memo(function Thumbnail({clip,seconds=0}:{clip:Clip;seconds?:number}){
  const ref=useRef<HTMLSpanElement>(null);const [src,setSrc]=useState<string>();
  useEffect(()=>{let live=true;const observer=new IntersectionObserver(entries=>{if(entries.some(e=>e.isIntersecting)){observer.disconnect();getThumbnail(clip.path,seconds).then(src=>{if(live)setSrc(src);}).catch(()=>{});}},{rootMargin:'100px'});if(ref.current)observer.observe(ref.current);return()=>{live=false;observer.disconnect();};},[clip.path,seconds]);
  return <span className="thumbnail" ref={ref}>{src?<img src={src} alt="" draggable={false}/>:<Film size={18}/>}</span>;
});
