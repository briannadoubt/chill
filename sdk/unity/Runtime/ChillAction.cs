using UnityEngine;

namespace Chill.Unity
{
    [DisallowMultipleComponent]
    [AddComponentMenu("Chill/Semantic Action")]
    public sealed class ChillAction : MonoBehaviour
    {
        [SerializeField]
        private string semanticName = "";

        [SerializeField]
        private string role = "control";

        [SerializeField]
        private ChillActionActivation activation = ChillActionActivation.Primary;

        [SerializeField]
        private ChillInput input = ChillInput.Unknown;

        public void Activate()
        {
            if (!ChillRuntime.IsConfigured)
            {
                return;
            }
            ChillRuntime.Action(
                semanticName,
                ChillContext.Resolve(transform),
                activation,
                input,
                role);
        }
    }
}
