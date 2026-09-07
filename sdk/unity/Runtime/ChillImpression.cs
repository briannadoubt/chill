using UnityEngine;

namespace Chill.Unity
{
    [DisallowMultipleComponent]
    [AddComponentMenu("Chill/Semantic Impression")]
    public sealed class ChillImpression : MonoBehaviour
    {
        [SerializeField]
        private string semanticName = "";

        [SerializeField]
        private string role = "content";

        private bool observed;

        private void OnEnable()
        {
            TryObserve();
        }

        private void Start()
        {
            TryObserve();
        }

        private void OnDisable()
        {
            observed = false;
        }

        private void TryObserve()
        {
            if (observed || !ChillRuntime.IsConfigured)
            {
                return;
            }
            observed = ChillRuntime.Impression(
                semanticName,
                ChillContext.Resolve(transform),
                role,
                1d);
        }
    }
}
