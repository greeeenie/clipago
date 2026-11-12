import {useEffect,useRef,useState} from 'react';
import {VideoPreview,type PreviewProps} from './VideoPreview';

export function Editor(props:PreviewProps){
 const player=useRef<HTMLDivElement>(null);
 const [fullscreen,setFullscreen]=useState(false);
 useEffect(()=>{
  const change=()=>setFullscreen(!!player.current&&document.fullscreenElement===player.current);
  document.addEventListener('fullscreenchange',change);
  return()=>document.removeEventListener('fullscreenchange',change);
 },[]);
 const toggleFullscreen=()=>{
  if(fullscreen)void document.exitFullscreen().catch(()=>{});
  else void player.current?.requestFullscreen().catch(()=>{});
 };
 return <div className="clip-player" ref={player}>
  <VideoPreview key={props.clip.path} {...props} showTimeline={!fullscreen&&props.showTimeline!==false} fullscreen={fullscreen} onFullscreen={toggleFullscreen}/>
 </div>;
}
