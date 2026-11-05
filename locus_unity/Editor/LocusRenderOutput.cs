using System;
using System.Collections.Generic;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;

namespace Locus.Rendering
{
    public enum RenderInputKind { PointerMove=1, PointerDown=2, PointerUp=3, Wheel=4, KeyDown=5, KeyUp=6, Cancel=7, Focus=8 }

    public struct RenderInput
    {
        public RenderInputKind Kind;
        public ulong Sequence, TimestampTicks;
        public uint Buttons, Modifiers, Key;
        // Top-left origin. Values can leave [0,1] while the pointer is captured.
        public Vector2 Position, Delta, SurfacePosition, TexturePosition;
        public Vector2Int SurfaceSize;
    }

    /// <summary>Publishes caller-rendered SDR textures. Owns transport resources,
    /// never the caller's scene, camera, animation, pipeline or source texture.</summary>
    [InitializeOnLoad]
    public sealed class RenderOutput : IDisposable
    {
        const string Dll="locus_unity_native";
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern int locus_render_version();
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern ulong locus_render_create(byte[] id,int length);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern int locus_render_bind(ulong handle,IntPtr texture);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern long locus_render_enqueue(ulong handle);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern void locus_render_cancel(UIntPtr packet);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern IntPtr locus_render_callback();
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern void locus_render_close(ulong handle);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern int locus_render_wait(ulong handle,uint timeout);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern int locus_render_input(ulong handle,out NativeInput input);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern int locus_render_ack_input(ulong handle,ulong sequence,ulong timestamp);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern int locus_render_state(ulong handle,out NativeState state);
        [DllImport(Dll,CallingConvention=CallingConvention.Cdecl)] static extern int locus_render_error(byte[] bytes,int capacity);
        [StructLayout(LayoutKind.Sequential)] struct NativeInput
        {
            public ulong sequence,timestamp;
            public uint kind,buttons,modifiers,key;
            public float x,y,dx,dy;
            public uint width,height;
            public ulong reserved;
        }
        [StructLayout(LayoutKind.Sequential)] struct NativeState
        {
            public ulong closed,attached,visible,width,height,request,published,dropped,errors,pending;
        }
        static readonly Dictionary<string,RenderOutput> Outputs=new Dictionary<string,RenderOutput>();
        static RenderOutput[] OutputSnapshot=new RenderOutput[0];
        static readonly int MainThreadId=Thread.CurrentThread.ManagedThreadId;
        static readonly Action SignalTick=CreateSignalTick();
        public static readonly string DomainGeneration=Guid.NewGuid().ToString("N");
        static RenderOutput()
        {
            EditorApplication.update+=PollAll;
            EditorApplication.quitting+=CloseAll;
            AssemblyReloadEvents.beforeAssemblyReload+=CloseAll;
            EditorApplication.playModeStateChanged+=OnPlayModeChange;
        }
        static Action CreateSignalTick()
        {
            var method=typeof(EditorApplication).GetMethod("SignalTick",BindingFlags.Static|BindingFlags.NonPublic);
            return method==null?null:(Action)Delegate.CreateDelegate(typeof(Action),method);
        }
        static void RequireMainThread()
        {
            if(Thread.CurrentThread.ManagedThreadId!=MainThreadId)throw new InvalidOperationException("RenderOutput must be used on the Unity main thread.");
        }
        static Exception Error()
        {
            var buffer=new byte[4096];int length=locus_render_error(buffer,buffer.Length);
            return new InvalidOperationException("Locus GPU output: "+Encoding.UTF8.GetString(buffer,0,Math.Max(0,length)));
        }
        public static void CheckSupport()
        {
            RequireMainThread();
            if(Application.platform!=RuntimePlatform.WindowsEditor||SystemInfo.graphicsDeviceType!=GraphicsDeviceType.Direct3D11)
                throw new NotSupportedException("GPU output currently requires Windows Editor with D3D11.");
            try {if(locus_render_version()!=1)throw new InvalidOperationException("GPU transport version mismatch. Update the Locus Unity plugin.");}
            catch(EntryPointNotFoundException){throw new InvalidOperationException("The native plugin predates GPU output support. Update the plugin and restart Unity.");}
            catch(DllNotFoundException){throw new InvalidOperationException("The Locus native plugin is missing. Install the Locus Unity plugin.");}
        }
        // The desktop host allocates and scopes ids before exposing them to View code.
        internal static RenderOutput Register(string id)
        {
            RequireMainThread();CheckSupport();Guid guid;
            if(!Guid.TryParseExact(id,"D",out guid))throw new ArgumentException("Invalid output id.");
            if(Outputs.ContainsKey(id))throw new InvalidOperationException("Output already registered.");
            var output=new RenderOutput(id);Outputs.Add(id,output);OutputSnapshot=new List<RenderOutput>(Outputs.Values).ToArray();return output;
        }
        /// <summary>Resolve the id returned by locus.unity.renderer.createOutput().</summary>
        public static RenderOutput Get(string id)
        {
            RequireMainThread();RenderOutput output;
            if(id==null||!Outputs.TryGetValue(id,out output)||output._disposed)throw new InvalidOperationException("GPU output is closed or belongs to an old Unity domain. Create a new output.");
            return output;
        }
        internal static void Close(string id) {RequireMainThread();RenderOutput output;if(id!=null&&Outputs.TryGetValue(id,out output))output.Dispose();}
        static void PollAll()
        {
            if(Outputs.Count==0)return;
            foreach(var output in OutputSnapshot)
            {
                try {output.Poll();}catch(Exception exception){Debug.LogException(exception);output.Dispose();}
            }
        }
        static void CloseAll() {foreach(var output in new List<RenderOutput>(Outputs.Values))output.Dispose();}
        static void OnPlayModeChange(PlayModeStateChange state)
        {
            // Also needed when Enter Play Mode Options disables domain reload.
            if(state==PlayModeStateChange.ExitingEditMode||state==PlayModeStateChange.ExitingPlayMode)CloseAll();
        }
        readonly ulong _handle;
        readonly IntPtr _callback;
        readonly CommandBuffer _commands;
        readonly Thread _wakeThread;
        RenderTexture _source;
        int _width,_height;
        bool _disposed;
        volatile bool _stopWake;
        ulong _lastRequest;
        public string Id {get;private set;}
        public bool IsVisible {get;private set;}
        public bool IsAttached {get;private set;}
        public Vector2Int RequestedSize {get;private set;}
        public ulong PublishedFrames {get;private set;}
        public ulong DroppedFrames {get;private set;}
        public event Action<RenderInput> Input;
        public event Action<Vector2Int> SizeRequested;
        public event Action<bool> VisibilityChanged;
        public event Action FrameRequested;
        public event Action Closed;
        RenderOutput(string id)
        {
            Id=id;var bytes=Encoding.UTF8.GetBytes(id);_handle=locus_render_create(bytes,bytes.Length);
            if(_handle==0)throw Error();
            _callback=locus_render_callback();_commands=new CommandBuffer{name="Locus GPU output "+id};
            if(SignalTick!=null)
            {
                _wakeThread=new Thread(WakeLoop){IsBackground=true,Name="Locus render input wake"};_wakeThread.Start();
            }
        }
        void WakeLoop()
        {
            // SignalTick is explicitly thread-safe in UnityCsReference. No other
            // Unity API or View callback runs on this waiter thread.
            try {while(!_stopWake){if(locus_render_wait(_handle,100)!=0&&!_stopWake)SignalTick();}}
            catch { _stopWake=true; }
        }
        void AssertLive(){RequireMainThread();if(_disposed)throw new ObjectDisposedException("RenderOutput");}
        public void InvalidateSource(){AssertLive();_source=null;}
        /// <summary>Publish after this frame's rendering has been submitted. The
        /// source must be single-sample RGBA8/BGRA8 SDR, without mipmaps. The SDK
        /// retains the native source resource until its queued copy is retired.</summary>
        public bool Publish(RenderTexture texture){return PublishInternal(texture,null);}
        /// <summary>Use from the View's SRP callback to append the copy to the same
        /// ScriptableRenderContext as its rendering. This does not submit or
        /// reconfigure that context; the View/pipeline retains ownership.</summary>
        public bool Publish(RenderTexture texture,ScriptableRenderContext context){return PublishInternal(texture,context);}
        bool PublishInternal(RenderTexture texture,ScriptableRenderContext? context)
        {
            AssertLive();if(texture==null||!texture.IsCreated())throw new ArgumentException("Publish requires a created RenderTexture.");
            if(texture!=_source||texture.width!=_width||texture.height!=_height)
            {
                // GetNativeTexturePtr can synchronize the render thread. Cache it
                // per source, not per frame. Call InvalidateSource after Release/
                // Create on the same managed RenderTexture object.
                if(locus_render_bind(_handle,texture.GetNativeTexturePtr())!=0)return HandlePublishFailure();
                _source=texture;_width=texture.width;_height=texture.height;
            }
            long token=locus_render_enqueue(_handle);if(token<0)return HandlePublishFailure();if(token==0)return false;
            try {
                _commands.Clear();_commands.IssuePluginEventAndData(_callback,1,new IntPtr(token));
                if(context.HasValue)context.Value.ExecuteCommandBuffer(_commands);else Graphics.ExecuteCommandBuffer(_commands);
                return true;
            }catch {locus_render_cancel(new UIntPtr((ulong)token));throw;}
        }
        bool HandlePublishFailure()
        {
            var error=Error();NativeState state;
            // The frontend can close between Poll and Publish on this editor
            // tick. Treat remote close as a lifecycle event, not a render fault.
            if(locus_render_state(_handle,out state)==0&&state.closed!=0){Dispose();return false;}
            throw error;
        }
        void Poll()
        {
            if(_disposed)return;NativeState state;
            if(locus_render_state(_handle,out state)!=0||state.closed!=0){Dispose();return;}
            if(state.errors!=0)throw new InvalidOperationException("Locus GPU output failed on the render thread.");
            IsAttached=state.attached!=0;PublishedFrames=state.published;DroppedFrames=state.dropped;
            var size=new Vector2Int((int)state.width,(int)state.height);
            if(size!=RequestedSize){RequestedSize=size;Dispatch(SizeRequested,size);}
            bool visible=state.visible!=0;if(visible!=IsVisible){IsVisible=visible;Dispatch(VisibilityChanged,visible);}
            NativeInput next;RenderInput? move=null;
            for(int i=0;i<256&&!_disposed&&locus_render_input(_handle,out next)>0;i++)
            {
                var input=new RenderInput{Kind=(RenderInputKind)next.kind,Sequence=next.sequence,TimestampTicks=next.timestamp,Buttons=next.buttons,Modifiers=next.modifiers,Key=next.key,Position=new Vector2(next.x,next.y),Delta=new Vector2(next.dx,next.dy),SurfacePosition=new Vector2(next.x*next.width,next.y*next.height),TexturePosition=new Vector2(next.x*_width,next.y*_height),SurfaceSize=new Vector2Int((int)next.width,(int)next.height)};
                if(input.Kind==RenderInputKind.PointerMove){if(move.HasValue)input.Delta+=move.Value.Delta;move=input;}
                else {if(move.HasValue){DispatchInput(move.Value);move=null;}if(!_disposed)DispatchInput(input);}
            }
            if(move.HasValue&&!_disposed)DispatchInput(move.Value);
            if(!_disposed&&state.request!=_lastRequest){_lastRequest=state.request;Dispatch(FrameRequested);}
        }
        void DispatchInput(RenderInput input)
        {
            // Poll may have read the next edge while coalescing moves. Mark the
            // event actually being delivered before a handler can Publish.
            if(locus_render_ack_input(_handle,input.Sequence,input.TimestampTicks)!=0)throw Error();
            Dispatch(Input,input);
        }
        static void Dispatch<T>(Action<T> handlers,T value){if(handlers!=null)foreach(Action<T> handler in handlers.GetInvocationList())try{handler(value);}catch(Exception e){Debug.LogException(e);}}
        static void Dispatch(Action handlers){if(handlers!=null)foreach(Action handler in handlers.GetInvocationList())try{handler();}catch(Exception e){Debug.LogException(e);}}
        public void Dispose()
        {
            RequireMainThread();if(_disposed)return;_disposed=true;_stopWake=true;
            locus_render_close(_handle);if(_wakeThread!=null)_wakeThread.Join(250);
            _commands.Release();_source=null;Outputs.Remove(Id);OutputSnapshot=new List<RenderOutput>(Outputs.Values).ToArray();IsVisible=false;IsAttached=false;
            Dispatch(Closed);Input=null;SizeRequested=null;VisibilityChanged=null;FrameRequested=null;Closed=null;
        }
    }
}
