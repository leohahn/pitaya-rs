using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Google.Protobuf;
using NPitaya.Metrics;
using NPitaya.Models;
using NPitaya.Serializer;
using NPitaya.Protos;
using static NPitaya.Utils.Utils;

namespace NPitaya
{
    public partial class PitayaCluster
    {
        static async Task<Response> HandleRpc(RpcClient rpcClient, Protos.Request req, RPCType type)
        {
            byte[] data = req.Msg.Data.ToByteArray();
            Route route = Route.FromString(req.Msg.Route);

            string handlerName = $"{route.service}.{route.method}";

            PitayaSession s = null;
            var response = new Response();

            RemoteMethod handler;
            if (type == RPCType.Sys)
            {
                s = new Models.PitayaSession(req.Session, rpcClient, req.FrontendID);
                if (!HandlersDict.ContainsKey(handlerName))
                {
                    response = GetErrorResponse("PIT-404",
                        $"handler not found! handler name: {handlerName}");
                    return response;
                }

                handler = HandlersDict[handlerName];
            }
            else
            {
                if (!RemotesDict.ContainsKey(handlerName))
                {
                    response = GetErrorResponse("PIT-404",
                        $"remote not found! remote name: {handlerName}");
                    return response;
                }

                handler = RemotesDict[handlerName];
            }

            Task ans;
            if (handler.ArgType != null)
            {
                var arg = _serializer.Unmarshal(data, handler.ArgType);
                if (type == RPCType.Sys)
                    ans = handler.Method.Invoke(handler.Obj, new[] {s, arg}) as Task;
                else
                    ans = handler.Method.Invoke(handler.Obj, new[] {arg}) as Task;
            }
            else
            {
                if (type == RPCType.Sys)
                    ans = handler.Method.Invoke(handler.Obj, new object[] {s}) as Task;
                else
                    ans = handler.Method.Invoke(handler.Obj, new object[] { }) as Task;
            }

            await ans;
            byte[] ansBytes;

            if (handler.ReturnType != typeof(void))
            {
                ansBytes = SerializerUtils.SerializeOrRaw(ans.GetType().
                    GetProperty("Result")
                    ?.GetValue(ans), _serializer);
            }
            else
            {
                ansBytes = new byte[]{};
            }
            response.Data = ByteString.CopyFrom(ansBytes);
            return response;
        }

        static void DispatchRpc(RpcClient rpcClient, IntPtr rpc, Protos.Request req)
        {
            Task.Run(async () => {
                var res = new Protos.Response();
                try
                {
                    res = await HandleRpc(rpcClient, req, req.Type);
                }
                catch (Exception e)
                {
                    res = GetErrorResponse("PIT-500", e.Message);

                    var innerMostException = e;
                    while (innerMostException.InnerException != null)
                        innerMostException = innerMostException.InnerException;

                    Logger.Error("Exception thrown in handler: {0}", innerMostException.Message);
#if NPITAYA_DEBUG
                    Logger.Error("StackTrace: {0}", e.StackTrace);
#endif
                }
                finally
                {
                    unsafe
                    {
                        byte[] responseData = res.ToByteArray();
                        Int32 responseLen = responseData.Length;

                        fixed (byte* p = responseData)
                        {
                            IntPtr err = pitaya_rpc_respond(rpc, (IntPtr)p, responseLen);
                            if (err != IntPtr.Zero)
                            {
                                pitaya_error_drop(err);
                                Logger.Error("Failed to respond to rpc");
                            }
                        }
                    }
                }
            });
        }

    }
}