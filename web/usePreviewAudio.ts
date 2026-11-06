import {useCallback,useEffect,useRef,useState,type RefObject} from 'react';
import {api,fileUrl,type Clip} from './api';

// Stream stems through media elements, so opening a long clip does not decode
// its entire audio into RAM. Only the foreground editor runs this mixer.
export function usePreviewAudio(video:RefObject<HTMLVideoElement|null>,clip:Clip,selected:number[],volume:number,muted:boolean){
  const players=useRef(new Map<number,HTMLAudioElement>());
  const latest=useRef({selected,volume,muted});latest.current={selected,volume,muted};
  const [ready,setReady]=useState(false);const [error,setError]=useState('');
  const separate=clip.audio_tracks.length>1;
  const allSelected=clip.audio_tracks.every(t=>selected.includes(t.index));
  const custom=separate&&(ready||!allSelected||clip.audio_mix_index===null);
  const loading=custom&&!ready&&selected.length>0&&!error;
  const resume=useRef(false);
  const cancelResume=useCallback(()=>{resume.current=false;},[]);
  useEffect(()=>{
    const el=video.current;if(!el)return;
    let live=true;let timer:ReturnType<typeof setInterval>|undefined;
    const pause=()=>{clearInterval(timer);timer=undefined;for(const audio of players.current.values())audio.pause();};
    const settings=()=>{for(const [index,audio] of players.current){audio.volume=latest.current.volume;audio.muted=latest.current.muted||!latest.current.selected.includes(index);}};
    const sync=(force=false)=>{
      settings();
      for(const audio of players.current.values()){
        if(Number.isFinite(audio.duration)&&audio.duration>0&&el.currentTime>=audio.duration){audio.pause();continue;}
        if(audio.readyState>0&&(force||Math.abs(audio.currentTime-el.currentTime)>.08))audio.currentTime=el.currentTime;
        audio.playbackRate=el.playbackRate;
        if(el.paused||el.seeking||el.ended)audio.pause();
        else if(audio.paused)void audio.play().catch(()=>{if(live){pause();el.pause();setError('Не удалось воспроизвести выбранные дорожки');}});
      }
    };
    const play=()=>{sync(true);clearInterval(timer);timer=setInterval(()=>sync(),200);};
    const seek=()=>{if(el.paused)sync(true);else play();};
    const stalled=()=>{pause();};
    el.addEventListener('playing',play);el.addEventListener('pause',pause);el.addEventListener('ended',pause);
    el.addEventListener('seeking',stalled);el.addEventListener('seeked',seek);el.addEventListener('ratechange',seek);
    el.addEventListener('waiting',stalled);
    if(separate)void api.previewAudio(clip.path).then(tracks=>{
      if(!live)return;
      for(const track of tracks){
        const audio=new Audio(fileUrl(track.path));audio.preload='auto';audio.muted=true;
        audio.addEventListener('loadedmetadata',()=>{if(live)sync(true);});
        audio.addEventListener('error',()=>{if(live){pause();el.pause();setError('Не удалось открыть аудиодорожку');}});
        players.current.set(track.index,audio);
      }
      setReady(true);
    }).catch(e=>{if(live)setError(String(e));});
    return()=>{
      live=false;if(!el.paused)el.pause();pause();
      el.removeEventListener('playing',play);el.removeEventListener('pause',pause);el.removeEventListener('ended',pause);
      el.removeEventListener('seeking',stalled);el.removeEventListener('seeked',seek);el.removeEventListener('ratechange',seek);el.removeEventListener('waiting',stalled);
      for(const audio of players.current.values()){audio.removeAttribute('src');audio.load();}players.current.clear();
    };
  },[clip.path,separate,video]);
  useEffect(()=>{
    const el=video.current;if(el){el.volume=volume;el.muted=muted||custom||selected.length===0;}
    for(const [index,audio] of players.current){audio.volume=volume;audio.muted=muted||!selected.includes(index);}
  },[selected,volume,muted,ready,custom,video]);
  useEffect(()=>{
    const el=video.current;if(!el)return;
    if(loading&&!el.paused){resume.current=true;el.pause();}
    if(ready){
      for(const audio of players.current.values()){
        if(audio.readyState>0)audio.currentTime=el.currentTime;
        if(!el.paused&&!(Number.isFinite(audio.duration)&&audio.duration>0&&el.currentTime>=audio.duration))void audio.play().catch(()=>{el.pause();setError('Не удалось воспроизвести выбранные дорожки');});
      }
      if(resume.current){resume.current=false;void el.play().catch(()=>{});}
    }
  },[ready,loading,video]);
  return {loading,error:custom?error:'',videoMuted:muted||custom||selected.length===0,cancelResume};
}
