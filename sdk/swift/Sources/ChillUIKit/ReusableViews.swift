#if os(iOS)
  import UIKit

  /// A table cell whose semantic element lifetime resets exactly when UIKit
  /// crosses the native reuse boundary.
  open class ChillTableViewCell: UITableViewCell {
    open override func prepareForReuse() {
      super.prepareForReuse()
      resetUIKitReusableState(of: self)
    }
  }

  /// A table header/footer whose semantic element lifetime follows reuse.
  open class ChillTableHeaderFooterView: UITableViewHeaderFooterView {
    open override func prepareForReuse() {
      super.prepareForReuse()
      resetUIKitReusableState(of: self)
    }
  }

  /// A collection cell whose semantic element lifetime resets exactly when
  /// UIKit crosses the native reuse boundary.
  open class ChillCollectionViewCell: UICollectionViewCell {
    open override func prepareForReuse() {
      super.prepareForReuse()
      resetUIKitReusableState(of: self)
    }
  }

  /// A supplementary collection view with the same reuse semantics as cells.
  open class ChillCollectionReusableView: UICollectionReusableView {
    open override func prepareForReuse() {
      super.prepareForReuse()
      resetUIKitReusableState(of: self)
    }
  }
#endif
