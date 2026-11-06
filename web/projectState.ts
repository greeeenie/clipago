import type {Clip,MovieProject,MovieSegment} from './api';
export function clipGroups(clips:Clip[]){
  const groups=new Map<string,{path:string;raw?:Clip;edited:Clip[]}>();
  for(const clip of clips.filter(c=>!c.is_movie)){
    const path=clip.original_path??clip.path;const group=groups.get(path)??{path,edited:[]};
    if(clip.original_path)group.edited.push(clip);else group.raw=clip;groups.set(path,group);
  }
  return [...groups.values()];
}
export function moveSegment(segments:MovieSegment[],from:string,to:number){const next=[...segments];const index=next.findIndex(s=>s.id===from);if(index<0)return next;const [item]=next.splice(index,1);next.splice(Math.max(0,Math.min(next.length,to)),0,item);return next;}
export function loadProject(key='rewind.movie.v1'):MovieProject{
  try {const value=JSON.parse(localStorage.getItem(key)??'null');if(value&&typeof value.name==='string'&&Array.isArray(value.segments)&&value.segments.length<=128&&value.segments.every((s:MovieSegment)=>typeof s.id==='string'&&typeof s.path==='string'&&Number.isFinite(s.start)&&Number.isFinite(s.end)&&s.end>s.start&&(s.gapBefore===undefined||(Number.isFinite(s.gapBefore)&&s.gapBefore>=0))&&s.config&&Array.isArray(s.config.audioTracks)))return value;}catch{}
  return {name:'Новый мувик',segments:[]};
}
