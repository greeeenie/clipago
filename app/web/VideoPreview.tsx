import {useEffect,useRef,useState} from 'react';
import {Play,Pause,Volume2,VolumeX,Maximize,LoaderCircle} from 'lucide-react';
import {Timeline,type Selection} from './Timeline';
import {IconButton,Slider} from './components';
import {api,fileUrl,time,type Clip} from './api';
import {usePreviewAudio} from './usePreviewAudio';
import {defaultEditorHotkeys,keyCombination} from './editorHotkeys';
import {listen} from '@tauri-apps/api/event';

export type PreviewProps={clip:Clip;selection:Selection;onChange:(s:Selection)=>void;compact?:boolean;busy?:boolean;suspended?:boolean;onSave?:()=>void;audioTracks?:number[];autoPlay?:boolean;onRangeEnd?:()=>void;onViewed?:()=>void;showTimeline?:boolean;displayTime?:{cursor:number;duration:number};onPlaybackChange?:(playing:boolean)=>void;keyboardPlayback?:boolean;playHotkey?:string};
export function VideoPreview({clip,selection,onChange,compact=false,busy=false,suspended=false,onSave,audioTracks,autoPlay=false,onRangeEnd,onViewed,showTimeline=true,displayTime,onPlaybackChange,keyboardPlayback=true,playHotkey=defaultEditorHotkeys.play}:PreviewProps){
  const video=useRef<HTMLVideoElement>(null);const host=useRef<HTMLDivElement>(null);const [bounds,setBounds]=useState({width:640,height:480});
  const [playing,setPlaying]=useState(false);const [muted,setMuted]=useState(false);const [src,setSrc]=useState(()=>fileUrl(clip.path));const [preparing,setPreparing]=useState(false);const [failure,setFailure]=useState('');const fallback=useRef(false);const latest=useRef(selection);latest.current=selection;
  const [volume,setVolume]=useState(()=>{try{const stored=localStorage.getItem('rewind.preview.volume');const value=stored===null?1:Number(stored);return Number.isFinite(value)?Math.max(0,Math.min(1,value)):1;}catch{return 1;}});
  const endCallback=useRef(onRangeEnd);endCallback.current=onRangeEnd;const endedRange=useRef(false);const wasAuto=useRef(false);const viewed=useRef(false);
  const rangeEnd=()=>{if(!endedRange.current){endedRange.current=true;endCallback.current?.();}};
  const restoreVolume=useRef(volume||1);
  const preview=usePreviewAudio(video,clip,audioTracks??clip.audio_tracks.map(t=>t.index),volume,muted);
  useEffect(()=>{try{localStorage.setItem('rewind.preview.volume',String(volume));}catch{}if(volume>0)restoreVolume.current=volume;},[volume]);
  useEffect(()=>{const el=video.current;if(!el)return;if(autoPlay&&!preview.loading&&el.readyState>0&&!busy&&!suspended)void el.play().catch(()=>{});else if(!autoPlay&&wasAuto.current){preview.cancelResume();el.pause();}wasAuto.current=autoPlay;},[autoPlay,preview.loading,busy,suspended]);
  const changeVolume=(value:number)=>{setVolume(value/100);if(value>0)setMuted(false);};
  const toggleMute=()=>{if(volume===0){setVolume(restoreVolume.current);setMuted(false);}else setMuted(!muted);};
  useEffect(()=>{const observer=new ResizeObserver(entries=>{const {width,height}=entries[0].contentRect;setBounds({width,height});});if(host.current)observer.observe(host.current);return()=>observer.disconnect();},[]);
  const toggle=()=>{const el=video.current;if(!el||busy||suspended||preview.loading)return;if(el.paused){if(el.currentTime>=selection.end-.03||el.currentTime<selection.start)el.currentTime=selection.start;onPlaybackChange?.(true);void el.play().catch(()=>{onPlaybackChange?.(false);setFailure('Не удалось воспроизвести видео');});}else{onPlaybackChange?.(false);el.pause();}};
  const seek=(next:Selection)=>{video.current?.pause();if(video.current)video.current.currentTime=next.cursor;onChange(next);};
  useEffect(()=>{const el=video.current;if(el&&!playing&&Math.abs(el.currentTime-selection.cursor)>.001)el.currentTime=selection.cursor;},[selection.cursor,playing]);
  useEffect(()=>{
    const key=(e:KeyboardEvent)=>{
      if(!keyboardPlayback||e.defaultPrevented||e.repeat||suspended)return;
      const target=e.target instanceof Element?e.target:null;
      if(target?.closest('input,textarea,select,[contenteditable="true"],[role="combobox"],[role="dialog"]'))return;
      const combination=keyCombination(e);
      if(compact&&combination==='Enter'&&playHotkey!=='Enter'){e.preventDefault();onSave?.();return;}
      if(!compact&&target?.closest('button,[role="slider"]'))return;
      if(combination===playHotkey){e.preventDefault();toggle();}
    };
    window.addEventListener('keydown',key);
    return()=>window.removeEventListener('keydown',key);
  });
  useEffect(()=>{if(busy||suspended){preview.cancelResume();video.current?.pause();}},[busy,suspended]);
  useEffect(()=>{if(!playing)return;let id=0;let last=0;const tick=(now:number)=>{const el=video.current;if(!el||el.paused)return;if(el.currentTime>=latest.current.end){el.pause();el.currentTime=latest.current.end;onChange({...latest.current,cursor:latest.current.end});rangeEnd();return;}if(now-last>40){last=now;onChange({...latest.current,cursor:el.currentTime});}id=requestAnimationFrame(tick);};id=requestAnimationFrame(tick);return()=>cancelAnimationFrame(id);},[playing,onChange]);
  useEffect(()=>{const hidden=()=>{if(document.hidden){preview.cancelResume();video.current?.pause();}};document.addEventListener('visibilitychange',hidden);return()=>document.removeEventListener('visibilitychange',hidden);},[]);
  useEffect(()=>{let live=true;let off:(()=>void)|undefined;void listen('app-hidden',()=>{preview.cancelResume();video.current?.pause();}).then(fn=>{if(live)off=fn;else fn();});return()=>{live=false;off?.();};},[]);
  const failed=()=>{if(fallback.current){setFailure('Видео недоступно для предпросмотра');return;}fallback.current=true;setPreparing(true);api.preview(clip.path).then(path=>setSrc(fileUrl(path))).catch(e=>setFailure(String(e))).finally(()=>setPreparing(false));};
  const aspect=Number.isFinite(clip.aspect_ratio)&&clip.aspect_ratio>0?clip.aspect_ratio:16/9;const height=Math.max(40,Math.min(bounds.width/aspect,bounds.height-(showTimeline?(compact?80:130):44)));const width=Math.min(bounds.width,height*aspect);const stackWidth=Math.max(width,Math.min(bounds.width,480));
  return <div className={`editor-workspace ${compact?'quick-editor':''}`} ref={host}>
    <div className="editor-stack" style={{width:stackWidth}}>
      <div className="video-box" style={{aspectRatio:aspect,width,alignSelf:'center'}}>
        <video ref={video} src={src} preload="metadata" playsInline muted={preview.videoMuted} onError={failed} onPlay={()=>{if(!viewed.current){viewed.current=true;onViewed?.();}endedRange.current=false;setPlaying(true);}} onPause={()=>setPlaying(false)} onEnded={()=>{setPlaying(false);rangeEnd();}} onClick={toggle} onLoadedMetadata={()=>{if(video.current){video.current.currentTime=latest.current.cursor;if(autoPlay&&!preview.loading&&!busy&&!suspended)void video.current.play().catch(()=>{});}}}/>
        {(preparing||preview.loading||failure||preview.error)&&<div className="video-message">{preparing||preview.loading?<><LoaderCircle className="spin" size={18}/>{preview.loading?'Подготовка звука…':'Подготовка предпросмотра…'}</>:failure||preview.error}</div>}
      </div>
      <div className="player-controls">
        <IconButton label={playing?'Пауза':'Воспроизвести'} onClick={toggle}>{playing?<Pause size={16}/>:<Play size={16}/>}</IconButton>
        <span className="player-time">{time(displayTime?.cursor??selection.cursor)} <span>/ {time(displayTime?.duration??clip.duration)}</span></span>
        <span className="spacer"/>
        <div className="player-volume"><IconButton label={muted||volume===0?'Включить звук':'Выключить звук'} disabled={!clip.has_audio} onClick={toggleMute}>{muted||volume===0?<VolumeX size={16}/>:<Volume2 size={16}/>}</IconButton><Slider label="Громкость предпросмотра" value={muted?0:Math.round(volume*100)} min={0} max={100} onChange={changeVolume}/><output aria-hidden="true">{muted?0:Math.round(volume*100)}%</output></div>
        <IconButton label="Полный экран" onClick={()=>void video.current?.requestFullscreen().catch(()=>{})}><Maximize size={15}/></IconButton>
      </div>
      {showTimeline&&<Timeline clip={clip} selection={selection} onChange={seek} compact={compact} disabled={busy}/>}
    </div>
  </div>;
}
