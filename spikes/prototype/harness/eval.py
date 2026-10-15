import json,sys,collections
truth=json.load(open('truth.json'))
viol={}
for l in open(sys.argv[1] if len(sys.argv)>1 else '../cachewitness/violations.jsonl'):
    r=json.loads(l); viol.setdefault(r['key'],r)
per=collections.defaultdict(lambda: collections.Counter())
lat=collections.defaultdict(list)
fp_bg=[k for k in viol if int(k.split(':')[1])>4000]
for k,t in truth.items():
    v=viol.get(k); got=v['cause'] if v else None
    per[t['scenario']][ 'correct' if got==t['expected'] else f'got {got}']+=1
    if v: lat[t['scenario']].append(v['detected_ms_after_commit'])
for s,c in per.items():
    n=sum(c.values()); ok=c['correct']
    l=sorted(lat[s]); med=l[len(l)//2] if l else '-'
    print(f"{s:28s} {ok}/{n} correct  {dict((k,v) for k,v in c.items() if k!='correct')}  median detect {med}ms")
print("background false positives:", len(fp_bg))
