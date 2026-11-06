import {describe,it,expect} from 'vitest';
import {snapBuffer,trimAt,time,defaultConfig,withPreset,type Clip} from './api';
const clip:Clip={path:'clip.mp4',duration:120,width:1920,height:1080,bytes:120_000_000,has_audio:true,audio_tracks:[{index:0,name:"Системный звук"}],audio_mix_index:null,source:null,fps:60,aspect_ratio:16/9,original_path:null,viewed:false,is_movie:false};
describe('буфер',()=>{
  it('имеет ровно 19 допустимых значений с шагом 5 секунд',()=>{
    const stops=new Set<number>();for(let n=0;n<=150;n+=.1){const value=snapBuffer(n);expect(value%5).toBe(0);expect(value).toBeGreaterThanOrEqual(30);expect(value).toBeLessThanOrEqual(120);stops.add(value);}expect([...stops]).toEqual(Array.from({length:19},(_,i)=>30+i*5));expect(snapBuffer(63)).toBe(65);expect(snapBuffer(87)).toBe(85);
  });
});
describe('точная обрезка',()=>{
  it('не позволяет границам пересекаться и оставляет хотя бы кадр',()=>{
    const start=trimAt('start',200,0,90,120,60);expect(start.start).toBeCloseTo(90-1/60);expect(start.end).toBe(90);
    const end=trimAt('end',-20,30,90,120,60);expect(end.end).toBeCloseTo(30+1/60);expect(end.cursor).toBe(end.end);
  });
  it('фиксируется по кадрам и корректно переносит минуты',()=>{
    expect(trimAt('seek',12.345,0,120,120,60).cursor).toBeCloseTo(12.35);expect(time(59.9996,true)).toBe('01:00.000');expect(trimAt('seek',999,0,120,120,60).cursor).toBe(120);
  });
});
describe('экспорт',()=>{
  it('переключает режимы без несовместимых параметров',()=>{
    const original=defaultConfig(clip,true);const copy=withPreset({...original,height:720},'copy',clip);
    expect(copy.quality).toBe('Copy');expect(copy.height).toBeNull();expect(copy.encoding.fps).toBeNull();expect(copy.encoding.hardware).toBe(false);
    const compact=withPreset(original,'compact',clip);expect(compact.bitrate).toBeLessThan(original.bitrate);expect(compact.quality).toBe('Bitrate');
  });
});
