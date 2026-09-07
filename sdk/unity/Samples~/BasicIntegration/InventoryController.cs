using Chill.Unity;
using UnityEngine;

namespace Chill.Unity.Samples
{
    public sealed class InventoryController : MonoBehaviour
    {
        public void Reload()
        {
            ChillActivityScope activity = ChillRuntime.BeginActivity(
                "inventory.load",
                null,
                ChillActivityKind.Storage);
            try
            {
                LoadInventory();
                activity.Succeed();
            }
            catch
            {
                activity.Fail("inventory_unavailable");
                throw;
            }
            finally
            {
                activity.Dispose();
            }
        }

        private static void LoadInventory()
        {
            // Application work belongs here. Chill does not inspect its values.
        }
    }
}
