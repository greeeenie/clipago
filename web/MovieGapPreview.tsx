import {useEffect,useRef,useState} from 'react';
import {Pause,Play} from 'lucide-react';
import {IconButton} from './components';
import {time} from './api';

export function MovieGapPreview({cursor,duration,playing,onToggle}:{cursor:number;duration:number;playing:boolean;onToggle:()=>void}){
 const host=useRef<HTMLDivElement>(null);const [bounds,setBounds]=useState({width:640,height:400});
 useEffect(()=>{if(typeof ResizeObserver==='undefined')return;const observer=new ResizeObserver(e=>setBounds({width:e[0].contentRect.width,height:e[0].contentRect.height}));if(host.current)observer.observe(host.current);return()=>observer.disconnect();},[]);
 const width=Math.max(40,Math.min(bounds.width,(bounds.height-44)*16/9));
 return <div className="editor-workspace" ref={host}><div className="editor-stack" style={{width:Math.max(width,Math.min(bounds.width,480))}}><div className="video-box gap-frame" aria-label="Чёрный промежуток" style={{width,aspectRatio:'16/9',alignSelf:'center'}} onClick={onToggle}/><div className="player-controls"><IconButton label={playing?'Пауза':'Воспроизвести'} onClick={onToggle}>{playing?<Pause size={16}/>:<Play size={16}/>}</IconButton><span className="player-time">{time(cursor)} <span>/ {time(duration)}</span></span></div></div></div>;
}
