"""Tile layouts for Oorbit's planets: Goldberg polyhedra GP(m, n).

    python3 -I plugins/oorbit/scripts/goldberg.py plugins/oorbit/game/layouts

Writes tiles_<N>.json with N tile centres (the vertices of a geodesic
icosahedron, unit vectors) and each tile's corner ring (the circumcentres of
the Delaunay triangles around it, counter-clockwise seen from outside). The
12 pentagons come first, then the hexagons from north to south. The game
draws the planet from these files; the simulation only needs the count
(tuning.tiles in crates/oorbit-sim/data/catalog.toml).

GP(1,1) has 32 tiles, GP(2,0) 42, GP(2,1) 72 (the one in use), GP(3,0) 92.
"""
import itertools, json, math, sys
def norm(v): l=math.sqrt(sum(x*x for x in v)); return [x/l for x in v]
def sub(a,b): return [a[i]-b[i] for i in range(3)]
def dot(a,b): return sum(a[i]*b[i] for i in range(3))
def cross(a,b): return [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]
t=(1+5**.5)/2
V=[norm(v) for v in [(-1,t,0),(1,t,0),(-1,-t,0),(1,-t,0),(0,-1,t),(0,1,t),(0,-1,-t),(0,1,-t),(t,0,-1),(t,0,1),(-t,0,-1),(-t,0,1)]]
F=[(0,11,5),(0,5,1),(0,1,7),(0,7,10),(0,10,11),(1,5,9),(5,11,4),(11,10,2),(10,7,6),(7,1,8),(3,9,4),(3,4,2),(3,2,6),(3,6,8),(3,8,9),(4,9,5),(2,4,11),(6,2,10),(8,6,7),(9,8,1)]
def points(m,n):
    e1=(1.0,0.0); e2=(0.5,math.sqrt(3)/2)
    c=lambda i,j:(i*e1[0]+j*e2[0], i*e1[1]+j*e2[1])
    P0,P1,P2=c(0,0),c(m,n),c(-n,m+n)
    det=(P1[0]-P0[0])*(P2[1]-P0[1])-(P2[0]-P0[0])*(P1[1]-P0[1])
    pts=[]
    R=m+n+1
    for (a,b,cc) in F:
        A,B,C=V[a],V[b],V[cc]
        for i in range(-R,R+1):
            for j in range(-R,R+1):
                x,y=c(i,j)
                l1=((x-P0[0])*(P2[1]-P0[1])-(P2[0]-P0[0])*(y-P0[1]))/det
                l2=((P1[0]-P0[0])*(y-P0[1])-(x-P0[0])*(P1[1]-P0[1]))/det
                l0=1-l1-l2
                if min(l0,l1,l2) < -1e-9: continue
                p=norm([l0*A[k]+l1*B[k]+l2*C[k] for k in range(3)])
                if not any(dot(p,q)>1-1e-9 for q in pts): pts.append(p)
    return pts
def layout(m,n):
    P=points(m,n); N=len(P)
    near=[sorted(range(N),key=lambda j:-dot(P[i],P[j]))[1:9] for i in range(N)]
    faces=set()
    for i in range(N):
        for j,k in itertools.combinations(near[i],2):
            tri=tuple(sorted((i,j,k)))
            if tri in faces: continue
            nrm=norm(cross(sub(P[j],P[i]),sub(P[k],P[i])))
            if dot(nrm,P[i])<0: nrm=[-x for x in nrm]
            d=dot(nrm,P[i])
            if all(dot(nrm,P[q])<=d+1e-9 for q in range(N)): faces.add(tri)
    centres=[]; rings=[]
    fc={f:norm([sum(P[v][k] for v in f) for k in range(3)]) for f in faces}
    # circumcentre on the sphere = normal of the face plane
    for f in faces:
        a,b,c=(P[v] for v in f); nrm=norm(cross(sub(b,a),sub(c,a)))
        if dot(nrm,a)<0: nrm=[-x for x in nrm]
        fc[f]=nrm
    order=sorted(range(N), key=lambda i:(sum(1 for f in faces if i in f)!=5, -P[i][1], math.atan2(P[i][2],P[i][0])))
    for i in order:
        ring=[fc[f] for f in faces if i in f]
        c=P[i]; up=[0,1,0] if abs(c[1])<0.9 else [1,0,0]
        u=norm(cross(c,up)); w=cross(c,u)
        ring.sort(key=lambda p: math.atan2(dot(sub(p,c),w), dot(sub(p,c),u)))
        centres.append(c); rings.append(ring)
    return {"centers":centres,"corners":rings}
for (m,n) in [(2,1)]:
    L=layout(m,n)
    sides=[len(r) for r in L["corners"]]
    print(m,n,len(L["centers"]),"pent",sides.count(5),"hex",sides.count(6),"other",[s for s in sides if s not in (5,6)])
    rounded={k:[[round(x,6) for x in p] if not isinstance(p[0],list) else [[round(x,6) for x in q] for q in p] for p in v] for k,v in L.items()}
    json.dump(rounded,open(f"{sys.argv[1]}/tiles_{len(L['centers'])}.json","w"),separators=(",",":"))
