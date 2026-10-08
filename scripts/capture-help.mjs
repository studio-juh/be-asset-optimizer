// Capture actual app screens with deterministic, generated demonstration assets.
// Start npm run dev, then run this script with Playwright installed or PLAYWRIGHT_MODULE set.
import { createRequire } from "node:module";
import { mkdir } from "node:fs/promises";
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ channel: "msedge", headless: true, args: ["--enable-unsafe-swiftshader"] });
try {
  const page = await browser.newPage({ viewport: { width: 1180, height: 700 }, deviceScaleFactor: 1 });
  page.setDefaultTimeout(15000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(() => {
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    const image = path => {
      const canvas = document.createElement("canvas"); canvas.width = canvas.height = 256;
      const context = canvas.getContext("2d"); const pixels = context.createImageData(256,256);
      for (let y=0;y<256;y++) for(let x=0;x<256;x++) {
        const i=(y*256+x)*4; const wave=Math.sin(x/10)*Math.cos(y/12);
        const normal=/normal|detail/.test(path);
        const gray=/roughness|ao|metallic|height/.test(path);
        const values=normal?[128+Math.cos(x/10)*40,128+Math.sin(y/12)*40,245]:gray?[170+wave*50,170+wave*50,170+wave*50]:[145+wave*35,95+wave*25,50+wave*15];
        pixels.data.set([...values.map(Math.round),255],i);
      }
      context.putImageData(pixels,0,0); return canvas.toDataURL();
    };
    const paths=["demo/albedo.png","demo/normal.png","demo/roughness.png","demo/metallic.png"];
    const cube = () => {
      const positions=[],normals=[],uvs=[];
      const faces=[[[1,-1,-1],[1,1,-1],[1,1,1],[1,-1,1]], [[-1,-1,1],[-1,1,1],[-1,1,-1],[-1,-1,-1]], [[-1,1,-1],[-1,1,1],[1,1,1],[1,1,-1]], [[-1,-1,1],[-1,-1,-1],[1,-1,-1],[1,-1,1]], [[-1,-1,1],[1,-1,1],[1,1,1],[-1,1,1]], [[1,-1,-1],[-1,-1,-1],[-1,1,-1],[1,1,-1]]];
      const ns=[[1,0,0],[-1,0,0],[0,1,0],[0,-1,0],[0,0,1],[0,0,-1]],tex=[[0,0],[1,0],[1,1],[0,1]];
      faces.forEach((face,f)=>[0,1,2,0,2,3].forEach(v=>{positions.push(...face[v]);normals.push(...ns[f]);uvs.push(...tex[v]);}));
      const data=new Float32Array([...positions,...normals,...uvs]);
      const gltf={asset:{version:"2.0"},buffers:[{uri:"data:application/octet-stream;base64,"+btoa(String.fromCharCode(...new Uint8Array(data.buffer))),byteLength:data.byteLength}],bufferViews:[{buffer:0,byteOffset:0,byteLength:432},{buffer:0,byteOffset:432,byteLength:432},{buffer:0,byteOffset:864,byteLength:288}],accessors:[{bufferView:0,componentType:5126,count:36,type:"VEC3",min:[-1,-1,-1],max:[1,1,1]},{bufferView:1,componentType:5126,count:36,type:"VEC3"},{bufferView:2,componentType:5126,count:36,type:"VEC2"}],images:[{uri:image("albedo")}],textures:[{source:0}],materials:[{pbrMetallicRoughness:{baseColorTexture:{index:0},metallicFactor:0,roughnessFactor:0.65}}],meshes:[{primitives:[{attributes:{POSITION:0,NORMAL:1,TEXCOORD_0:2},material:0}]}],nodes:[{mesh:0}],scenes:[{nodes:[0]}],scene:0};
      return btoa(JSON.stringify(gltf));
    };
    window.__TAURI_INTERNALS__={invoke:async(cmd,args)=>{
      if(cmd==="create_preview")return image(args.path);
      if(cmd==="plugin:dialog|open")return args.options.multiple?paths:paths[0];
      if(cmd==="inspect_files")return {files:args.paths.map(path=>({path,name:path.split('/').pop(),originalBytes:524288,width:1024,height:1024})),failures:[],truncated:false};
      if(cmd==="load_model_package")return {path:args.path,name:"demo-cube.glb",bytesBase64:cube(),fileBytes:7000,textures:[],scanLimited:false};
      if(cmd==="diagnose_textures")return [];
      return null;
    },transformCallback:()=>1,unregisterCallback:()=>{},convertFileSrc:path=>path};
  });
  for(const lang of ["ja","en"]) {
    const dir=new URL(`../public/help/${lang}/`,import.meta.url); await mkdir(dir,{recursive:true});
    for(const tab of ["optimize","normal","atlas","preview","material","channels","adjust"]) {
      if (process.env.HELP_CAPTURE_TABS && !process.env.HELP_CAPTURE_TABS.split(",").includes(tab)) continue;
      await page.goto(`${process.env.HELP_CAPTURE_URL || "http://127.0.0.1:1420"}/?storePreview=1&lang=${lang}&tab=${tab}`);
      const panel=page.locator(".tool-panel:not([hidden])");
      if(tab==="optimize"||tab==="normal")await panel.getByRole("button",{name:lang==="ja"?"ファイルを追加":"Add Files",exact:true}).click();
      else if(tab==="atlas")await page.evaluate(()=>window.dispatchEvent(new CustomEvent("smartpng-atlas-drag",{detail:{type:"drop",paths:["demo/albedo.png","demo/normal.png","demo/roughness.png"],position:{x:700,y:250}}})));
      else if(tab==="preview") {await panel.locator("canvas").waitFor();await page.evaluate(()=>window.dispatchEvent(new CustomEvent("smartpng-model-drop",{detail:["demo/cube.glb"]})));await panel.getByRole("button",{name:lang==="ja"?"アセット診断":"Diagnose Asset"}).click();}
      else if(tab==="material") {await panel.locator("canvas").waitFor();for(const [key,path] of [["albedo","demo/albedo.png"],["normal","demo/normal.png"],["roughness","demo/roughness.png"]]){await panel.locator(`[data-material-map="${key}"]`).hover();await page.evaluate(path=>window.dispatchEvent(new CustomEvent("smartpng-material-drop",{detail:[path]})),path);}}
      else if(tab==="channels") {await panel.locator(".texture-input").first().waitFor();await page.evaluate(()=>window.dispatchEvent(new CustomEvent("smartpng-channels-drop",{detail:["demo/ao.png","demo/roughness.png","demo/metallic.png"]})));await panel.getByRole("button",{name:"ORM",exact:true}).click();}
      else {await panel.locator(".texture-input").first().waitFor();await page.evaluate(()=>window.dispatchEvent(new CustomEvent("smartpng-adjust-drop",{detail:["demo/normal.png","demo/detail.png"]})));await panel.locator(".normal-preview-pane:last-child img").waitFor();}
      await page.waitForTimeout(450);
      await page.screenshot({path:new URL(`${tab}.png`,dir).pathname.replace(/^\/(\w:)/,"$1")});
      console.log(`${lang}/${tab}.png`);
    }
  }
  if(errors.length)throw Error(errors.join("\n"));
} finally { await browser.close(); }
