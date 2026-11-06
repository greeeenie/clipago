import type {Clip,MovieProject,MovieSegment} from './api';

export function movieTiming(project:MovieProject,clips:Clip[]){
 const fps=Math.min(240,Math.max(1,project.output?.encoding.fps??Math.max(project.segments.length?1:30,...project.segments.map(s=>Math.round(clips.find(c=>c.path===s.path)?.fps??30)))));
 let duration=0;
 const parts=project.segments.map(segment=>{
  const gap=Math.round(Math.max(0,segment.gapBefore??0)*fps)/fps;
  const start=duration+gap;const length=Math.max(1,Math.round((segment.end-segment.start)*fps))/fps;
  duration=start+length;return {segment,start,end:duration,duration:length};
 });
 return {parts,duration,fps};
}
export type MovieTiming=ReturnType<typeof movieTiming>;
export type TimedPart=MovieTiming['parts'][number];

export function resolveMovieTime(timing:MovieTiming,time:number){
 const cursor=Math.max(0,Math.min(timing.duration,time));
 const part=timing.parts.find(p=>cursor>=p.start-1e-7&&cursor<p.end-1e-7)??(Math.abs(cursor-timing.duration)<1e-7?timing.parts.at(-1):undefined);
 return part?{part,cursor,sourceTime:Math.min(part.segment.end,part.segment.start+Math.max(0,cursor-part.start))}:undefined;
}

export function segmentsAtPositions(parts:TimedPart[]):MovieSegment[]{
 let end=0;
 return parts.map(p=>{const gapBefore=Math.max(0,p.start-end);end=p.end;return {...p.segment,gapBefore};});
}

// A single track never overlaps: dropping into a clip moves following clips
// forward. Empty time left behind stays in place until the user closes it.
export function moveSegmentPlacement(timing:MovieTiming,id:string,time:number,snapDistance=0,playhead?:number):{parts:TimedPart[];snapTime?:number}{
 const moving=timing.parts.find(p=>p.segment.id===id);if(!moving)return {parts:timing.parts};
 const rest=timing.parts.filter(p=>p!==moving);
 let start=Math.max(0,Math.round(time*timing.fps)/timing.fps);
 let snapTime:number|undefined;
 if(snapDistance>0){
  const points=[0,...rest.flatMap(p=>[p.start,p.end]),...(playhead!==undefined?[playhead]:[])];
  let distance=snapDistance+1e-7;let nearest=start;
  for(const point of points){
   const target=Math.round(point*timing.fps)/timing.fps;
   for(const candidate of [target,target-moving.duration]){
    if(candidate<0)continue;const delta=Math.abs(candidate-start);
    if(delta<distance){distance=delta;nearest=candidate;snapTime=target;}
   }
  }
  start=nearest;
 }
 let index=rest.findIndex(p=>p.start>=start-1e-7);if(index<0)index=rest.length;
 if(index>0)start=Math.max(start,rest[index-1].end);
 const result=[...rest];result.splice(index,0,{...moving,start,end:start+moving.duration});
 let end=0;
 const parts=result.map(p=>{const nextStart=Math.max(end,p.start);end=nextStart+p.duration;return {...p,start:nextStart,end};});
 const placed=parts.find(p=>p.segment.id===id)!;
 if(snapTime!==undefined&&Math.abs(placed.start-snapTime)>1e-7&&Math.abs(placed.end-snapTime)>1e-7)snapTime=undefined;
 return {parts,snapTime};
}

export function splitSegment(segment:MovieSegment,time:number,fps:number):MovieSegment[]|null{
 const rate=Math.max(1,fps);const point=Math.round(time*rate)/rate;const frame=1/rate;
 if(point-segment.start<frame-1e-6||segment.end-point<frame-1e-6)return null;
 return [{...segment,end:point},{...segment,id:crypto.randomUUID(),start:point,gapBefore:0,config:structuredClone(segment.config)}];
}
